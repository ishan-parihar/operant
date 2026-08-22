import re
P="crates/operant-core/src/agent/mod.rs"
src=open(P).read(); lines=src.split("\n")
def L(n): return lines[n-1]
CUTS=[("builders",567),("events",1102),("run",1439),("prompting",3047),
      ("compress",3565),("stream",3733),("TAIL",4676)]
for n,a in {567:"fn tool_allowed_by_allowlist",1102:"async fn emit(",1439:"async fn attempt_grace_call(",
            3047:"fn load_context_file_prompt(",3565:"async fn compress_context_overflow(",
            3733:"async fn process_stream(",4676:"struct ThinkBlockRouter"}.items():
    assert a in L(n),(n,L(n)[:70])
def ext(s):
    j=s
    while j>1:
        st=L(j-1).strip()
        if st=="" or not(st.endswith(";") or st.endswith("}") or st.endswith("{")):
            j-=1; continue
        break
    return j
E=[(n,ext(l)) for n,l in CUTS]
TESTS=None
for i,l in enumerate(lines,1):
    if l.startswith("mod tests"): TESTS=i; break
assert TESTS
uses=[];i=0
while i<60:
    l=lines[i]
    if re.match(r'^use\b',l):
        uses.append(l)
        while not l.rstrip().endswith(";"):
            i+=1;l=lines[i];uses.append(l)
    i+=1
USES="\n".join(uses)+"\n"
def widen(text):
    out=[];mode=None
    hdr=re.compile(r'^    (?:(pub(\(crate\))? )?(async )?fn |(pub(\(crate\))? )?(struct|enum|impl|const|static|type)\b)')
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
            if mode=='impl_inherent' and re.match(r'^    (?=(?:async\s+)?fn\b)',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
            if mode=='other': pass
        out.append(ln)
    return "\n".join(out)
for k in range(6):
    name=E[k][0]
    s=E[k][1]; e=E[k+1][1]
    body="\n".join(lines[s-1:e-1]).rstrip()
    if name=="stream":
        assert body.endswith("}")
        body=body[:-1].rstrip()
    fname=f"crates/operant-core/src/agent/{name}.rs"
    open(fname,"w").write(
      f"//! `{name}` — method-group impl block extracted verbatim from agent/mod.rs.\n\n"
      +USES+"\nuse super::*;\n\n"+widen("impl OperantAgent {\n"+body+"\n}")+"\n")
tail_s=E[-1][1]
head=lines[:E[0][1]-1]
txt="\n".join(head)
txt=txt.replace("impl OperantAgent {\n\n#[derive(Debug, Default)]","#[derive(Debug, Default)]",1)
head=txt.split("\n")
new=head+lines[tail_s-1:]
open(P,"w").write("\n".join(new).rstrip()+"\n")
print("agent split done; mod.rs now",len(new),"lines")
