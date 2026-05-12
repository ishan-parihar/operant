//! Yuanbao (Tencent QQ group chat) platform tools.
//!
//! Provides 5 tools for interacting with the Yuanbao platform via a
//! JSON-RPC-like HTTP API with HMAC-SHA256 authentication:
//!
//! - `yb_query_group_info`   — get group metadata
//! - `yb_query_group_members` — list group members
//! - `yb_send_dm`             — send a direct message to a user
//! - `yb_search_sticker`      — search the built-in sticker library
//! - `yb_send_sticker`        — send a sticker to a chat
//!
//! # Authentication
//!
//! Each request signs a canonical string `nonce + timestamp + app_id + app_secret`
//! with HMAC-SHA256 (key = `app_secret`).  The hex-encoded signature is sent in the
//! `X-YB-Auth` header.  `app_id` is sent in `X-YB-App-Id`.
//!
//! # Sticker catalogue
//!
//! The built-in sticker map is embedded directly so that `yb_search_sticker` and
//! `yb_send_sticker` work without extra network calls.

use std::sync::Arc;
use std::time::{SystemTime, UNIX_EPOCH};

use async_trait::async_trait;
use schemars::JsonSchema;
use serde::{Deserialize, Serialize};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};

use crate::schema::ToolSchema;
use crate::tools::{HermesTool, ToolContext, ToolResult};

// ---------------------------------------------------------------------------
// HMAC-SHA256 (standalone — no external `hmac` crate needed)
// ---------------------------------------------------------------------------

fn hmac_sha256(key: &[u8], data: &[u8]) -> [u8; 32] {
    const BLOCK_SIZE: usize = 64;

    let mut key = key.to_vec();
    if key.len() > BLOCK_SIZE {
        key = Sha256::digest(&key).to_vec();
    }
    key.resize(BLOCK_SIZE, 0);

    let mut ipad = vec![0x36u8; BLOCK_SIZE];
    let mut opad = vec![0x5cu8; BLOCK_SIZE];
    for i in 0..BLOCK_SIZE {
        ipad[i] ^= key[i];
        opad[i] ^= key[i];
    }

    let mut inner = Sha256::new();
    inner.update(&ipad);
    inner.update(data);
    let inner_hash = inner.finalize();

    let mut outer = Sha256::new();
    outer.update(&opad);
    outer.update(inner_hash);
    outer.finalize().into()
}

fn hex_encode(data: &[u8]) -> String {
    const HEX: &[u8] = b"0123456789abcdef";
    let mut out = String::with_capacity(data.len() * 2);
    for &b in data {
        out.push(HEX[(b >> 4) as usize] as char);
        out.push(HEX[(b & 0x0f) as usize] as char);
    }
    out
}

fn now_secs() -> u64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .unwrap_or_default()
        .as_secs()
}

// ---------------------------------------------------------------------------
// Shared client
// ---------------------------------------------------------------------------

/// Shared HTTP client + credentials for the Yuanbao REST API.
struct YuanbaoClient {
    http: reqwest::Client,
    base_url: String,
    app_id: String,
    app_secret: String,
}

impl YuanbaoClient {
    fn new(base_url: String, app_id: String, app_secret: String) -> Self {
        Self {
            http: reqwest::Client::new(),
            base_url,
            app_id,
            app_secret,
        }
    }

    /// Build the HMAC-SHA256 hex signature for a request.
    fn signature(&self, nonce: &str, timestamp: &str) -> String {
        let canonical = format!("{}{}{}{}", nonce, timestamp, self.app_id, self.app_secret);
        let sig = hmac_sha256(self.app_secret.as_bytes(), canonical.as_bytes());
        hex_encode(&sig)
    }

    /// POST to `{base_url}/{endpoint}` with a JSON-RPC-like body.
    async fn call(&self, endpoint: &str, params: Value) -> ToolResult {
        let nonce = uuid::Uuid::new_v4().to_string();
        let timestamp = now_secs().to_string();
        let sig = self.signature(&nonce, &timestamp);

        let url = format!("{}/{}", self.base_url.trim_end_matches('/'), endpoint);
        let payload = json!({ "method": endpoint, "params": params });

        match self
            .http
            .post(&url)
            .header("Content-Type", "application/json")
            .header("X-YB-Auth", &sig)
            .header("X-YB-App-Id", &self.app_id)
            .header("X-YB-Timestamp", &timestamp)
            .header("X-YB-Nonce", &nonce)
            .json(&payload)
            .send()
            .await
        {
            Ok(resp) => {
                let status = resp.status();
                match resp.json::<Value>().await {
                    Ok(body) => {
                        if !status.is_success() {
                            let msg = body
                                .get("message")
                                .and_then(|v| v.as_str())
                                .unwrap_or("request failed");
                            return ToolResult::error(
                                endpoint,
                                format!("API error ({}): {}", status.as_u16(), msg),
                            );
                        }
                        ToolResult::success(endpoint, body)
                    }
                    Err(e) => {
                        ToolResult::error(endpoint, format!("Failed to parse response: {}", e))
                    }
                }
            }
            Err(e) => ToolResult::error(endpoint, format!("HTTP request failed: {}", e)),
        }
    }
}

// ---------------------------------------------------------------------------
// Sticker catalogue (embedded — ported from yuanbao_sticker.py)
// ---------------------------------------------------------------------------

struct StickerEntry {
    name: &'static str,
    sticker_id: &'static str,
    package_id: &'static str,
    description: &'static str,
}

const STICKERS: &[StickerEntry] = &[
    StickerEntry {
        name: "六六六",
        sticker_id: "278",
        package_id: "1003",
        description: "666 厉害 牛 棒 绝了 好强 awesome",
    },
    StickerEntry {
        name: "我想开了",
        sticker_id: "262",
        package_id: "1003",
        description: "想开 佛系 释怀 顿悟 看淡了 无所谓",
    },
    StickerEntry {
        name: "害羞",
        sticker_id: "130",
        package_id: "1003",
        description: "腼腆 不好意思 脸红 娇羞 羞涩 捂脸",
    },
    StickerEntry {
        name: "比心",
        sticker_id: "252",
        package_id: "1003",
        description: "笔芯 爱你 爱心手势 love heart 喜欢你",
    },
    StickerEntry {
        name: "委屈",
        sticker_id: "125",
        package_id: "1003",
        description: "难过 想哭 可怜巴巴 瘪嘴 受伤 被欺负",
    },
    StickerEntry {
        name: "亲亲",
        sticker_id: "146",
        package_id: "1003",
        description: "么么 mua 亲一下 kiss 飞吻 啵",
    },
    StickerEntry {
        name: "酷",
        sticker_id: "131",
        package_id: "1003",
        description: "帅 墨镜 cool 高冷 有型 swagger",
    },
    StickerEntry {
        name: "睡",
        sticker_id: "145",
        package_id: "1003",
        description: "睡觉 困 zzZ 打盹 躺平 休眠 sleepy",
    },
    StickerEntry {
        name: "发呆",
        sticker_id: "152",
        package_id: "1003",
        description: "懵 愣住 放空 呆滞 出神 脑子空白",
    },
    StickerEntry {
        name: "可怜",
        sticker_id: "157",
        package_id: "1003",
        description: "卖萌 求饶 委屈巴巴 弱小 拜托 眼巴巴",
    },
    StickerEntry {
        name: "摊手",
        sticker_id: "200",
        package_id: "1003",
        description: "无奈 没办法 耸肩 随便 那咋整 whatever",
    },
    StickerEntry {
        name: "头大",
        sticker_id: "213",
        package_id: "1003",
        description: "头疼 烦恼 郁闷 难搞 崩溃 一团乱",
    },
    StickerEntry {
        name: "吓",
        sticker_id: "256",
        package_id: "1003",
        description: "害怕 惊恐 震惊 吓一跳 恐怖 怂",
    },
    StickerEntry {
        name: "吐血",
        sticker_id: "203",
        package_id: "1003",
        description: "无语 崩溃 被雷 内伤 一口老血 屮",
    },
    StickerEntry {
        name: "哼",
        sticker_id: "185",
        package_id: "1003",
        description: "傲娇 生气 不满 撇嘴 不理 赌气",
    },
    StickerEntry {
        name: "嘿嘿",
        sticker_id: "220",
        package_id: "1003",
        description: "坏笑 猥琐笑 偷笑 憨笑 得意 你懂的",
    },
    StickerEntry {
        name: "头秃",
        sticker_id: "218",
        package_id: "1003",
        description: "程序员 加班 焦虑 没头发 秃了 肝爆",
    },
    StickerEntry {
        name: "暗中观察",
        sticker_id: "221",
        package_id: "1003",
        description: "窥屏 潜水 偷偷看 角落 围观 屏住呼吸",
    },
    StickerEntry {
        name: "我酸了",
        sticker_id: "224",
        package_id: "1003",
        description: "嫉妒 柠檬精 羡慕 吃柠檬 眼红 恰柠檬",
    },
    StickerEntry {
        name: "打call",
        sticker_id: "246",
        package_id: "1003",
        description: "应援 加油 支持 喝彩 助威 call",
    },
    StickerEntry {
        name: "庆祝",
        sticker_id: "251",
        package_id: "1003",
        description: "祝贺 开心 耶 party 胜利 干杯",
    },
    StickerEntry {
        name: "奋斗",
        sticker_id: "151",
        package_id: "1003",
        description: "努力 加油 拼搏 冲 干劲 卷起来",
    },
    StickerEntry {
        name: "惊讶",
        sticker_id: "143",
        package_id: "1003",
        description: "震惊 哇 不敢相信 OMG 居然 这么离谱",
    },
    StickerEntry {
        name: "疑问",
        sticker_id: "144",
        package_id: "1003",
        description: "问号 不懂 啥 为什么 啥情况 懵逼问",
    },
    StickerEntry {
        name: "仔细分析",
        sticker_id: "248",
        package_id: "1003",
        description: "思考 推敲 认真 研究 琢磨 让我想想",
    },
    StickerEntry {
        name: "撅嘴",
        sticker_id: "184",
        package_id: "1003",
        description: "嘟嘴 卖萌 不高兴 撒娇 嘴翘",
    },
    StickerEntry {
        name: "泪奔",
        sticker_id: "199",
        package_id: "1003",
        description: "大哭 伤心 破防 感动哭 泪流满面 呜呜",
    },
    StickerEntry {
        name: "尊嘟假嘟",
        sticker_id: "276",
        package_id: "1003",
        description: "真的假的 真假 可爱问 你骗我 是不是",
    },
    StickerEntry {
        name: "略略略",
        sticker_id: "113",
        package_id: "1003",
        description: "调皮 吐舌 不服 略 气死你 鬼脸",
    },
    StickerEntry {
        name: "困",
        sticker_id: "180",
        package_id: "1003",
        description: "想睡 倦 打哈欠 睁不开眼 好困啊 sleepy",
    },
    StickerEntry {
        name: "折磨",
        sticker_id: "181",
        package_id: "1003",
        description: "难受 痛苦 煎熬 蚌埠住了 受不了 要命",
    },
    StickerEntry {
        name: "抠鼻",
        sticker_id: "182",
        package_id: "1003",
        description: "不屑 无聊 淡定 无所谓 鄙视 挖鼻",
    },
    StickerEntry {
        name: "鼓掌",
        sticker_id: "183",
        package_id: "1003",
        description: "拍手 叫好 赞同 666 喝彩 掌声",
    },
    StickerEntry {
        name: "斜眼笑",
        sticker_id: "204",
        package_id: "1003",
        description: "滑稽 坏笑 doge 意味深长 阴阳怪气 嘿嘿嘿",
    },
    StickerEntry {
        name: "辣眼睛",
        sticker_id: "216",
        package_id: "1003",
        description: "看不下去 cringe 毁三观 太丑了 瞎了",
    },
    StickerEntry {
        name: "哦哟",
        sticker_id: "217",
        package_id: "1003",
        description: "惊讶 起哄 哇哦 有戏 不简单 哟",
    },
    StickerEntry {
        name: "吃瓜",
        sticker_id: "222",
        package_id: "1003",
        description: "围观 看戏 八卦 路人 看热闹 板凳",
    },
    StickerEntry {
        name: "狗头",
        sticker_id: "225",
        package_id: "1003",
        description: "doge 保命 开玩笑 滑稽 反讽 懂的都懂",
    },
    StickerEntry {
        name: "敬礼",
        sticker_id: "227",
        package_id: "1003",
        description: "salute 尊重 收到 遵命 致敬 报告",
    },
    StickerEntry {
        name: "哦",
        sticker_id: "231",
        package_id: "1003",
        description: "知道了 明白 敷衍 嗯 这样啊 收到",
    },
    StickerEntry {
        name: "拿到红包",
        sticker_id: "236",
        package_id: "1003",
        description: "红包 谢谢老板 发财 开心 抢到了 欧气",
    },
    StickerEntry {
        name: "牛吖",
        sticker_id: "239",
        package_id: "1003",
        description: "牛 厉害 强 666 佩服 大佬",
    },
    StickerEntry {
        name: "贴贴",
        sticker_id: "272",
        package_id: "1003",
        description: "抱抱 亲昵 蹭蹭 亲密 靠靠 撒娇贴",
    },
    StickerEntry {
        name: "爱心",
        sticker_id: "138",
        package_id: "1003",
        description: "心 love 喜欢你 红心 示爱 么么哒",
    },
    StickerEntry {
        name: "晚安",
        sticker_id: "170",
        package_id: "1003",
        description: "好梦 睡了 night 早点休息 安啦 moon",
    },
    StickerEntry {
        name: "太阳",
        sticker_id: "176",
        package_id: "1003",
        description: "晴天 早上好 阳光 morning 好天气 日",
    },
    StickerEntry {
        name: "柠檬",
        sticker_id: "266",
        package_id: "1003",
        description: "酸 嫉妒 柠檬精 羡慕 我酸 恰柠檬",
    },
    StickerEntry {
        name: "大冤种",
        sticker_id: "267",
        package_id: "1003",
        description: "倒霉 吃亏 自嘲 好心没好报 背锅 工具人",
    },
    StickerEntry {
        name: "吐了",
        sticker_id: "132",
        package_id: "1003",
        description: "恶心 yue 受不了 嫌弃 想吐 生理不适",
    },
    StickerEntry {
        name: "怒",
        sticker_id: "134",
        package_id: "1003",
        description: "生气 愤怒 火大 暴躁 气炸 怼",
    },
    StickerEntry {
        name: "玫瑰",
        sticker_id: "165",
        package_id: "1003",
        description: "花 示爱 表白 浪漫 送你花 情人节",
    },
    StickerEntry {
        name: "凋谢",
        sticker_id: "119",
        package_id: "1003",
        description: "花谢 失恋 难过 枯萎 心碎 凉了",
    },
    StickerEntry {
        name: "点赞",
        sticker_id: "159",
        package_id: "1003",
        description: "赞 认同 好棒 good like 大拇指 顶",
    },
    StickerEntry {
        name: "握手",
        sticker_id: "164",
        package_id: "1003",
        description: "合作 你好 商务 hello deal 成交 友好",
    },
    StickerEntry {
        name: "抱拳",
        sticker_id: "163",
        package_id: "1003",
        description: "谢谢 失敬 江湖 承让 拜托 有礼",
    },
    StickerEntry {
        name: "ok",
        sticker_id: "169",
        package_id: "1003",
        description: "好的 收到 没问题 okay 行 可以 懂了",
    },
    StickerEntry {
        name: "拳头",
        sticker_id: "174",
        package_id: "1003",
        description: "加油 干 冲 fight 力量 击拳 硬气",
    },
    StickerEntry {
        name: "鞭炮",
        sticker_id: "191",
        package_id: "1003",
        description: "过年 喜庆 爆竹 春节 噼里啪啦 红",
    },
    StickerEntry {
        name: "烟花",
        sticker_id: "258",
        package_id: "1003",
        description: "庆典 漂亮 新年 嘭 绽放 节日快乐",
    },
];

fn find_sticker_by_name(name: &str) -> Option<&'static StickerEntry> {
    let q = name.trim();
    // Exact match first
    for s in STICKERS {
        if s.name == q {
            return Some(s);
        }
    }
    // Substring match on name
    for s in STICKERS {
        if s.name.contains(q) || q.contains(s.name) {
            return Some(s);
        }
    }
    // Substring match on description
    for s in STICKERS {
        if s.description.contains(q) {
            return Some(s);
        }
    }
    None
}

fn find_sticker_by_id(id: &str) -> Option<&'static StickerEntry> {
    let q = id.trim();
    STICKERS.iter().find(|s| s.sticker_id == q)
}

fn random_sticker() -> &'static StickerEntry {
    let idx = (now_secs() as usize) % STICKERS.len();
    &STICKERS[idx]
}

fn search_stickers(query: &str, limit: usize) -> Vec<Value> {
    let limit = limit.max(1).min(50);
    let q = query.trim().to_lowercase();

    if q.is_empty() {
        return STICKERS.iter().take(limit).map(to_sticker_json).collect();
    }

    // Score each sticker and sort
    let mut scored: Vec<(f64, &StickerEntry)> = STICKERS
        .iter()
        .map(|s| {
            let name_lower = s.name.to_lowercase();
            let desc_lower = s.description.to_lowercase();

            let mut score = 0.0f64;
            if name_lower == q {
                score = 100.0;
            } else if name_lower.contains(&q) || q.contains(&name_lower) {
                score = 92.0;
            } else if desc_lower.contains(&q) {
                score = 80.0;
            } else if s.sticker_id == q {
                score = 100.0;
            }

            (score, s)
        })
        .collect();

    scored.sort_by(|a, b| b.0.partial_cmp(&a.0).unwrap_or(std::cmp::Ordering::Equal));

    // Filter by threshold
    let threshold = if scored.first().map(|s| s.0).unwrap_or(0.0) >= 22.0 {
        18.0
    } else if scored.first().map(|s| s.0).unwrap_or(0.0) >= 12.0 {
        10.0
    } else {
        6.0
    };

    let results: Vec<Value> = scored
        .into_iter()
        .filter(|(s, _)| *s >= threshold)
        .take(limit)
        .map(|(_, s)| to_sticker_json(s))
        .collect();

    if results.is_empty() {
        STICKERS.iter().take(limit).map(to_sticker_json).collect()
    } else {
        results
    }
}

fn to_sticker_json(s: &StickerEntry) -> Value {
    json!({
        "sticker_id": s.sticker_id,
        "name": s.name,
        "description": s.description,
        "package_id": s.package_id,
    })
}

// ---------------------------------------------------------------------------
// Tool 1: yb_query_group_info
// ---------------------------------------------------------------------------

pub struct YuanbaoQueryGroupInfo {
    client: Arc<YuanbaoClient>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct QueryGroupInfoArgs {
    group_code: String,
}

#[async_trait]
impl HermesTool for YuanbaoQueryGroupInfo {
    fn name(&self) -> &str {
        "yb_query_group_info"
    }

    fn description(&self) -> &str {
        "Query basic info about a group (called '派/Pai' in the app), including group name, owner, and member count."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<QueryGroupInfoArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: QueryGroupInfoArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        if args.group_code.is_empty() {
            return ToolResult::error(self.name(), "group_code is required");
        }

        self.client
            .call("query_group_info", json!({ "group_code": args.group_code }))
            .await
    }
}

// ---------------------------------------------------------------------------
// Tool 2: yb_query_group_members
// ---------------------------------------------------------------------------

pub struct YuanbaoQueryGroupMembers {
    client: Arc<YuanbaoClient>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct QueryGroupMembersArgs {
    group_code: String,
    #[serde(default = "default_member_action")]
    action: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    mention: bool,
}

fn default_member_action() -> String {
    "list_all".to_string()
}

#[async_trait]
impl HermesTool for YuanbaoQueryGroupMembers {
    fn name(&self) -> &str {
        "yb_query_group_members"
    }

    fn description(&self) -> &str {
        "Query members of a group (called '派/Pai' in the app). Use this tool when you need to @mention someone, \
         find a user by name, list bots (including Yuanbao AI), or list all members. \
         IMPORTANT: You MUST call this tool before @mentioning any user, because you need \
         the exact nickname to construct the @mention format."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<QueryGroupMembersArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: QueryGroupMembersArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        if args.group_code.is_empty() {
            return ToolResult::error(self.name(), "group_code is required");
        }

        self.client
            .call(
                "get_group_member_list",
                json!({
                    "group_code": args.group_code,
                    "action": args.action,
                    "name": args.name,
                    "mention": args.mention,
                }),
            )
            .await
    }
}

// ---------------------------------------------------------------------------
// Tool 3: yb_send_dm
// ---------------------------------------------------------------------------

pub struct YuanbaoSendDm {
    client: Arc<YuanbaoClient>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SendDmArgs {
    #[serde(default)]
    group_code: String,
    #[serde(default)]
    name: String,
    #[serde(default)]
    message: String,
    #[serde(default)]
    user_id: String,
    #[serde(default)]
    media_files: Option<Vec<MediaFileItem>>,
}

#[derive(Debug, Serialize, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct MediaFileItem {
    path: String,
    #[serde(default)]
    is_voice: bool,
}

#[async_trait]
impl HermesTool for YuanbaoSendDm {
    fn name(&self) -> &str {
        "yb_send_dm"
    }

    fn description(&self) -> &str {
        "Send a private/direct message (DM) to a user in a group, with optional media files. \
         This tool automatically looks up the user by name in the group member list and sends \
         the message. Use this when someone asks to privately message / 私信 / DM a user. \
         Supports text, images, and file attachments."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SendDmArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: SendDmArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        if args.message.is_empty() && args.media_files.is_none() {
            return ToolResult::error(self.name(), "message or media_files is required");
        }

        self.client
            .call(
                "send_dm",
                json!({
                    "group_code": args.group_code,
                    "name": args.name,
                    "message": args.message,
                    "user_id": args.user_id,
                    "media_files": args.media_files,
                }),
            )
            .await
    }
}

// ---------------------------------------------------------------------------
// Tool 4: yb_search_sticker
// ---------------------------------------------------------------------------

pub struct YuanbaoSearchSticker {
    client: Arc<YuanbaoClient>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SearchStickerArgs {
    #[serde(default)]
    query: String,
    #[serde(default = "default_sticker_limit")]
    limit: i32,
}

fn default_sticker_limit() -> i32 {
    10
}

#[async_trait]
impl HermesTool for YuanbaoSearchSticker {
    fn name(&self) -> &str {
        "yb_search_sticker"
    }

    fn description(&self) -> &str {
        "Search the built-in Yuanbao sticker (TIM face / 表情包) catalogue by keyword. \
         Returns the top matching candidates with sticker_id, name, and description. \
         Use this BEFORE yb_send_sticker to discover the right sticker_id. \
         Sticker = 贴纸 = TIM face — NOT a message reaction."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SearchStickerArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: SearchStickerArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        let results = search_stickers(&args.query, args.limit as usize);

        ToolResult::success(
            self.name(),
            json!({
                "success": true,
                "query": args.query,
                "count": results.len(),
                "results": results,
            }),
        )
    }
}

// ---------------------------------------------------------------------------
// Tool 5: yb_send_sticker
// ---------------------------------------------------------------------------

pub struct YuanbaoSendSticker {
    client: Arc<YuanbaoClient>,
}

#[derive(Debug, Deserialize, JsonSchema)]
#[serde(rename_all = "camelCase")]
struct SendStickerArgs {
    #[serde(default)]
    sticker: String,
    #[serde(default)]
    chat_id: String,
    #[serde(default)]
    reply_to: String,
}

#[async_trait]
impl HermesTool for YuanbaoSendSticker {
    fn name(&self) -> &str {
        "yb_send_sticker"
    }

    fn description(&self) -> &str {
        "Send a built-in sticker (TIMFaceElem / 贴纸表情) to the current Yuanbao chat. \
         Call yb_search_sticker first if you don't know the sticker_id/name. \
         Sticker = 贴纸 = TIM face — NOT a message reaction. \
         CRITICAL: Whenever the user asks you to send a sticker / 贴纸 / 表情包, you MUST \
         use this tool. DO NOT draw a PNG via code execution."
    }

    fn schema(&self) -> ToolSchema {
        ToolSchema::from_type::<SendStickerArgs>(self.name(), self.description())
    }

    async fn execute(&self, args: Value, _context: ToolContext) -> ToolResult {
        let args: SendStickerArgs = match serde_json::from_value(args) {
            Ok(a) => a,
            Err(e) => return ToolResult::error(self.name(), format!("Invalid arguments: {}", e)),
        };

        // Resolve sticker locally
        let raw = args.sticker.trim();
        let sticker_entry = if raw.is_empty() {
            Some(random_sticker())
        } else if raw.chars().all(|c| c.is_ascii_digit()) {
            find_sticker_by_id(raw).or_else(|| find_sticker_by_name(raw))
        } else {
            find_sticker_by_name(raw)
        };

        let entry = match sticker_entry {
            Some(e) => e,
            None => {
                return ToolResult::error(
                    self.name(),
                    format!(
                        "Sticker not found: {:?}. Use yb_search_sticker first to discover available stickers.",
                        raw,
                    ),
                );
            }
        };

        self.client
            .call(
                "send_sticker",
                json!({
                    "sticker_id": entry.sticker_id,
                    "name": entry.name,
                    "package_id": entry.package_id,
                    "chat_id": args.chat_id,
                    "reply_to": args.reply_to,
                }),
            )
            .await
    }
}

// ---------------------------------------------------------------------------
// Factory
// ---------------------------------------------------------------------------

/// Create all 5 Yuanbao tools sharing the same HTTP client + credentials.
///
/// Returns `None` if the required env vars are missing.
pub fn create_yuanbao_tools(
    base_url: String,
    app_id: String,
    app_secret: String,
) -> Vec<Box<dyn HermesTool>> {
    let client = Arc::new(YuanbaoClient::new(base_url, app_id, app_secret));

    vec![
        Box::new(YuanbaoQueryGroupInfo {
            client: client.clone(),
        }),
        Box::new(YuanbaoQueryGroupMembers {
            client: client.clone(),
        }),
        Box::new(YuanbaoSendDm {
            client: client.clone(),
        }),
        Box::new(YuanbaoSearchSticker {
            client: client.clone(),
        }),
        Box::new(YuanbaoSendSticker { client }),
    ]
}

#[cfg(test)]
mod tests {
    use super::*;

    // ---- hex_encode tests ------------------------------------------------

    #[test]
    fn test_hex_encode_empty() {
        assert_eq!(hex_encode(b""), "");
    }

    #[test]
    fn test_hex_encode_single_bytes() {
        assert_eq!(hex_encode(b"\x00"), "00");
        assert_eq!(hex_encode(b"\xff"), "ff");
        assert_eq!(hex_encode(b"\xab"), "ab");
    }

    #[test]
    fn test_hex_encode_known_string() {
        assert_eq!(hex_encode(b"hello"), "68656c6c6f");
        assert_eq!(hex_encode(b"deadbeef"), "6465616462656566");
    }

    #[test]
    fn test_hex_encode_raw_bytes() {
        assert_eq!(hex_encode(&[0xde, 0xad, 0xbe, 0xef]), "deadbeef");
        assert_eq!(hex_encode(&[0x00, 0x01, 0x02, 0xff]), "000102ff");
    }

    // ---- hmac_sha256 tests ------------------------------------------------

    #[test]
    fn test_hmac_sha256_known_value() {
        // RFC 4231 test case 2
        let key = b"Jefe";
        let data = b"what do ya want for nothing?";
        let result = hmac_sha256(key, data);
        let expected = "5bdcc146bf60754e6a042426089575c75a003f089d2739839dec58b964ec3843";
        assert_eq!(hex_encode(&result), expected);
    }

    #[test]
    fn test_hmac_sha256_deterministic() {
        let result1 = hmac_sha256(b"key", b"data");
        let result2 = hmac_sha256(b"key", b"data");
        assert_eq!(result1, result2);
    }

    #[test]
    fn test_hmac_sha256_different_keys() {
        let result1 = hmac_sha256(b"key1", b"data");
        let result2 = hmac_sha256(b"key2", b"data");
        assert_ne!(result1, result2);
    }

    #[test]
    fn test_hmac_sha256_different_data() {
        let result1 = hmac_sha256(b"key", b"data1");
        let result2 = hmac_sha256(b"key", b"data2");
        assert_ne!(result1, result2);
    }

    #[test]
    fn test_hmac_sha256_empty_key() {
        let result = hmac_sha256(b"", b"test data");
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_hmac_sha256_empty_data() {
        let result = hmac_sha256(b"test key", b"");
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_hmac_sha256_long_key() {
        // Key longer than BLOCK_SIZE (64) triggers key hashing first
        let key = vec![0xabu8; 128];
        let result = hmac_sha256(&key, b"test data");
        assert_eq!(result.len(), 32);
    }

    #[test]
    fn test_hmac_sha256_output_size() {
        // Output must always be 32 bytes (SHA-256 output)
        for size in [0, 1, 16, 64, 256] {
            let key = vec![0x42u8; size];
            let data = vec![0x99u8; size];
            let result = hmac_sha256(&key, &data);
            assert_eq!(result.len(), 32, "failed for key/data size {size}");
        }
    }

    // ---- find_sticker_by_name tests ---------------------------------------

    #[test]
    fn test_find_sticker_by_name_exact() {
        let s = find_sticker_by_name("六六六");
        assert!(s.is_some());
        assert_eq!(s.unwrap().sticker_id, "278");
    }

    #[test]
    fn test_find_sticker_by_name_substring() {
        let s = find_sticker_by_name("六六");
        assert!(s.is_some());
    }

    #[test]
    fn test_find_sticker_by_name_description_match() {
        let s = find_sticker_by_name("awesome");
        assert!(s.is_some());
    }

    #[test]
    fn test_find_sticker_by_name_not_found() {
        let s = find_sticker_by_name("xyznonexistentsticker12345");
        assert!(s.is_none());
    }

    #[test]
    fn test_find_sticker_by_name_whitespace_trimmed() {
        let s = find_sticker_by_name("  六六六  ");
        assert!(s.is_some());
        assert_eq!(s.unwrap().sticker_id, "278");
    }

    // ---- find_sticker_by_id tests -----------------------------------------

    #[test]
    fn test_find_sticker_by_id_exact() {
        let s = find_sticker_by_id("278");
        assert!(s.is_some());
        assert_eq!(s.unwrap().name, "六六六");
    }

    #[test]
    fn test_find_sticker_by_id_not_found() {
        let s = find_sticker_by_id("99999");
        assert!(s.is_none());
    }

    #[test]
    fn test_find_sticker_by_id_empty() {
        let s = find_sticker_by_id("");
        assert!(s.is_none());
    }

    #[test]
    fn test_find_sticker_by_id_whitespace() {
        let s = find_sticker_by_id("  278  ");
        assert!(s.is_some());
    }

    // ---- search_stickers tests --------------------------------------------

    #[test]
    fn test_search_stickers_empty_query() {
        let results = search_stickers("", 3);
        assert_eq!(results.len(), 3);
    }

    #[test]
    fn test_search_stickers_clamps_limit_low() {
        let results = search_stickers("", 0);
        assert_eq!(results.len(), 1);
    }

    #[test]
    fn test_search_stickers_clamps_limit_high() {
        let results = search_stickers("", 100);
        assert_eq!(results.len(), 50);
    }

    #[test]
    fn test_search_stickers_exact_name_match_ranked_first() {
        let results = search_stickers("六六六", 10);
        assert!(!results.is_empty());
        assert_eq!(results[0]["name"], "六六六");
    }

    #[test]
    fn test_search_stickers_no_match_falls_back_to_top() {
        let results = search_stickers("xyznonexistentquery", 5);
        assert_eq!(results.len(), 5);
    }

    #[test]
    fn test_search_stickers_by_sticker_id() {
        let results = search_stickers("278", 10);
        assert!(!results.is_empty());
    }

    #[test]
    fn test_search_stickers_query_is_lowercased() {
        let results = search_stickers("AWESOME", 5);
        assert!(!results.is_empty());
    }

    // ---- to_sticker_json tests --------------------------------------------

    #[test]
    fn test_to_sticker_json_format() {
        let entry = StickerEntry {
            name: "test",
            sticker_id: "123",
            package_id: "456",
            description: "a test sticker",
        };
        let json = to_sticker_json(&entry);
        assert_eq!(json["name"], "test");
        assert_eq!(json["sticker_id"], "123");
        assert_eq!(json["package_id"], "456");
        assert_eq!(json["description"], "a test sticker");
    }

    // ---- default value functions ------------------------------------------

    #[test]
    fn test_default_member_action_value() {
        assert_eq!(default_member_action(), "list_all");
    }

    #[test]
    fn test_default_sticker_limit_value() {
        assert_eq!(default_sticker_limit(), 10);
    }
}
