import re,os
P="crates/operant-config/src/schema/core.rs"
src=open(P).read(); lines=src.split("\n")
def L(n): return lines[n-1]
# boundary anchors (1-based line numbers)
A={"proxy_io":1622,"platform":1748,"tunnels":2398,"chan1":2558,"security":2918,
   "chan2":3144,"ops":3384,"config_impl":3573,"hasprop_end":5683,"tests":5683}
checks={1622:"pub(crate) trait AsyncReadWrite",1748:"pub struct PostgresMemoryConfig",
 2398:"pub struct TunnelConfig",2558:"pub(crate) struct ConfigWrapper",
 2918:"pub struct SecurityConfig",3144:"pub struct WeComConfig",
 3384:"pub struct CloudOpsConfig",3573:"impl Default for Config",
 5683:"mod tests"}
for n,a in checks.items():
    assert a in L(n),(n,L(n)[:60],a)
TESTS=5683
def ext(s):
    j=s
    while j>1:
        st=L(j-1).strip()
        if st=="" or not(st.endswith(";") or st.endswith("}") or st.endswith("{")):
            j-=1; continue
        break
    return j
E={k:ext(v) for k,v in A.items()}
E["hasprop_end"]=ext(5683)  # tests start ext
RANGES=[("proxy_io","proxy.rs",E["proxy_io"],E["platform"]),
 ("platform","platform_cfg.rs",E["platform"],E["tunnels"]),
 ("tunnels","tunnels.rs",E["tunnels"],E["chan1"]),
 ("chan1","channels_cfg.rs",E["chan1"],E["security"]),
 ("security","security_cfg.rs",E["security"],E["chan2"]),
 ("chan2","channels_cfg.rs",E["chan2"],E["ops"]),
 ("ops","ops_cfg.rs",E["ops"],E["config_impl"]),
 ("config_impl","config_impl.rs",E["config_impl"],E["hasprop_end"])]
# USES header: multi-line-aware use capture from core.rs head
uses=[];i=0
while i<40:
    l=lines[i]
    if re.match(r'^use\b',l):
        uses.append(l)
        while not l.rstrip().endswith(";"):
            i+=1;l=lines[i];uses.append(l)
    i+=1
USES="\n".join(uses)+"\n"
HDRS=2  # doc lines at top? find first non-doc
def widen(text):
    out=[];mode=None
    hdr=re.compile(r'^(?:pub(?:\((?:crate\))?\))?\s+)?(struct|enum|impl|fn|const|static|type|trait)\b')
    for ln in text.split("\n"):
        if ln.strip()=="": out.append(ln);continue
        if ln==ln.lstrip():
            m=hdr.match(ln)
            if m:
                k=m.group(1)
                mode=('struct' if k=='struct' else 'enum' if k=='enum'
                      else ('impl_trait' if " for " in ln.split("{")[0] else 'impl_inherent') if k=='impl' else None)
            elif not ln.startswith(("#","///","//","}")): mode=None
            if not ln.startswith("pub") and re.match(r'^(async fn |fn |struct |enum |const |static |type |trait )',ln):
                out.append("pub(crate) "+ln);continue
        else:
            if mode=='struct' and re.match(r'^    ([a-z_][a-z0-9_]*)\s*:',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
            if mode=='impl_inherent' and re.match(r'^    (?=(?:async\s+)?fn\b)',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
        out.append(ln)
    return "\n".join(out)
chunks={}
merge_channels=[]
for name,fname,s,e in RANGES:
    body=widen("\n".join(lines[s-1:e-1]).rstrip())
    if fname=="channels_cfg.rs": merge_channels.append(body); continue
    chunks[fname]=(name,body)
OUT="crates/operant-config/src/schema/"
for fname,(name,body) in chunks.items():
    open(OUT+fname,"w").write(
      f"//! `{name}` — extracted verbatim from the former schema/core.rs monolith.\n"
      "//! Re-exported from `schema` so every path is unchanged.\n\n"
      +USES+"\nuse super::*;\n\n"+body+"\n")
# merge into channels_cfg.rs BEFORE its own tests mod if present
cc=open(OUT+"channels_cfg.rs").read()
ins=len(cc)
m=re.search(r'^#\[cfg\(test\)\]\s*^mod tests \{',cc,re.M)
if m: ins=m.start()
merged="\n\n".join("// ---- channel configs relocated from core.rs (dedup pass 4) ----"==x and x or x for x in merge_channels)
cc=cc[:ins]+"\n\n"+merged+"\n\n"+cc[ins:]
open(OUT+"channels_cfg.rs","w").write(cc)
# tests file
inner=lines[TESTS-1:]
assert inner[0].strip().startswith("mod tests"),inner[0][:40]
body_t=inner[1:]
while body_t and body_t[-1].strip()=="": body_t.pop()
assert body_t[-1].strip()=="}"
open(OUT+"core_tests.rs","w").write("//! Core config tests (verbatim body of the former inline `mod tests`).\n"+"\n".join(body_t[:-1]).rstrip()+"\n")
# rebuild core.rs: keep [1..proxy_io_ext) + [hasprop_end..tests_ext)
new=lines[:E["proxy_io"]-1]
new+=lines[E["hasprop_end"]-1:E["tests"]-1] if E["hasprop_end"]!=E["tests"] else []
new+=["#[cfg(test)]","mod tests;"]
open(P,"w").write("\n".join(new).rstrip()+"\n")
print("core split done; core.rs now ~",len(new),"lines")
