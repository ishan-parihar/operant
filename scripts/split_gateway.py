import re
P="crates/operant-core/src/gateway/mod.rs"
src=open(P).read(); lines=src.split("\n")
def L(n): return lines[n-1]
CUTS=[("types",30),("CORE",769),("telegram",1322),("discord",2802),
      ("slack",3298),("webhook",3459),("whatsapp",3901),("email",4065),
      ("sms",4332),("admin",4467),("TAIL",4583)]
for n,a in {30:"pub struct GatewayConfig",769:"pub struct Gateway {",1322:"pub struct TelegramAdapter",
            2802:"pub struct DiscordAdapter",3298:"pub struct SlackAdapter",3459:"pub struct WebhookAdapter",
            3901:"pub struct WhatsAppAdapter",4065:"pub struct EmailAdapter",4332:"pub struct SmsAdapter",
            4467:"pub async fn handle_admin_command(",4583:"mod tests"}.items():
    assert a in L(n),(n,L(n)[:60])
def ext(s):
    j=s
    while j>1:
        st=L(j-1).strip()
        if st=="" or not(st.endswith(";") or st.endswith("}") or st.endswith("{")):
            j-=1; continue
        break
    return j
E=[(n,ext(l)) for n,l in CUTS]
TESTS=4583
uses=[];i=0
while i<30:
    l=lines[i]
    if re.match(r'^use\b',l):
        uses.append(l)
        while not l.rstrip().endswith(";"):
            i+=1;l=lines[i];uses.append(l)
    i+=1
USES="\n".join(uses)+"\n"
def widen(text):
    out=[];mode=None
    for ln in text.split("\n"):
        if ln.strip()=="": out.append(ln);continue
        if ln==ln.lstrip():
            if ln.startswith("impl"):
                mode="impl_trait" if " for " in ln.split("{")[0] else 'impl_inherent'
            elif re.match(r'^(?:pub(\(crate\))? )?(struct|enum|const|static|type)\b',ln): mode='other'
            else: mode=None
            if not ln.startswith("pub") and re.match(r'^(async fn |fn |struct |enum |const |static |type )',ln):
                out.append("pub(crate) "+ln);continue
        else:
            if mode=='struct' and re.match(r'^    ([a-z_][a-z0-9_]*)\s*:',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
            if mode=='impl_inherent' and re.match(r'^    (?=(?:async\s+)?fn\b)',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
        out.append(ln)
    return "\n".join(out)
for k in range(len(E)-1):
    name=E[k][0]
    if name in ("CORE","TAIL"): continue
    s=E[k][1]; e=E[k+1][1]
    body=widen("\n".join(lines[s-1:e-1]).rstrip())
    open(f"crates/operant-core/src/gateway/{name}.rs","w").write(
      f"//! `{name}` — extracted verbatim from gateway/mod.rs.\n\n"
      +USES+"\nuse super::*;\n\n"+body+"\n")
# rebuild mod.rs: head..core_start + core..tail_start(ext of tests)
head_end=E[0][1]   # types start
core_s=[v for n,v in E if n=="CORE"][0]
tail_s=E[-1][1]
new=lines[:head_end-1]+lines[core_s-1:tail_s-1]
decls=["// Platform adapters extracted verbatim from this file (dedup pass 6).",
       "mod admin;","mod discord;","mod email;","mod slack;","mod sms;","mod telegram;",
       "mod types;","mod webhook;","mod whatsapp;"]
decls+=[f"pub use {m}::*;" for m in ["admin","discord","email","slack","sms","telegram","types","webhook","whatsapp"]]
insert_at=len(lines[:head_end-1])
new=new[:insert_at]+decls+new[insert_at:]
open(P,"w").write("\n".join(new).rstrip()+"\n")
print("gateway split done; mod.rs now",len(new),"lines")
