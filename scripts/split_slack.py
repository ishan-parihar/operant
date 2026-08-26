import re,os
P="crates/operant-channels/src/slack.rs"
src=open(P).read(); lines=src.split("\n")
def L(n): return lines[n-1]
for n,a in {65:"const SLACK_HISTORY_MAX_RETRIES",163:"impl SlackChannel {",
            3274:"const SLACK_TRUNCATION_INDICATOR",3330:"impl Channel for SlackChannel {",
            4177:"mod tests"}.items():
    assert a in L(n),(n,L(n)[:60])
def ext(s):
    j=s
    while j>1:
        st=L(j-1).strip()
        if st=="" or not(st.endswith(";") or st.endswith("}") or st.endswith("{")):
            j-=1; continue
        break
    return j
h_ext,i_ext,c_ext,t_ext,tests_ext=ext(65),ext(163),ext(3274),ext(3330),ext(4177)
uses=[];i=0
while i<19:
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
            elif re.match(r'^(?:pub(\(crate\))? )?struct\b',ln): mode='struct'
            elif re.match(r'^(?:pub(\(crate\))? )?(enum|const|static|type)\b',ln): mode='other'
            elif re.match(r'^(?:pub(\(crate\))? )?(fn |async fn |use |mod |trait |#)',ln) or ln.startswith("}"):
                mode=None
            # col0 string-literal content: leave mode intact
            if not ln.startswith("pub") and re.match(r'^(async fn |fn |struct |enum |const |static |type )',ln):
                out.append("pub(crate) "+ln);continue
        else:
            if mode=='struct' and re.match(r'^    ([a-z_][a-z0-9_]*)\s*:',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
            if mode=='impl_inherent' and re.match(r'^    (?=(?:async\s+)?fn\b)',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
        out.append(ln)
    return "\n".join(out)
D="crates/operant-channels/src/slack"
os.makedirs(D,exist_ok=True)
open(f"{D}/helpers.rs","w").write(
 "//! Slack tunables, permalink/emoji helpers extracted verbatim.\n\n"
 +USES+"\nuse super::*;\n\n"+widen("\n".join(lines[h_ext-1:i_ext-1]).rstrip())+"\n")
open(f"{D}/channel_impl.rs","w").write(
 "//! Inherent methods on `SlackChannel` extracted verbatim.\n\n"
 +USES+"\nuse super::*;\n\n"+widen("\n".join(lines[i_ext-1:c_ext-1]).rstrip()).replace("super::transcription::","crate::transcription::")+"\n")
body_t=lines[tests_ext:]
mi=next(i for i,l in enumerate(body_t) if l.strip()=="mod tests {")
inner=body_t[mi+1:]
while inner and inner[-1].strip()=="": inner.pop()
assert inner[-1].strip()=="}"
open(f"{D}/tests.rs","w").write(
 "//! Slack channel tests (verbatim body of the former inline `mod tests`).\n"+"\n".join(inner[:-1]).rstrip()+"\n")
mod_new=(lines[:h_ext-1]
 +["","// Split modules (dedup pass 8) - paths unchanged.","mod channel_impl;","mod helpers;",
   "#[cfg(test)]","mod tests;","","pub(crate) use helpers::*;",""]
 +lines[c_ext-1:tests_ext-1])
open(P,"w").write("\n".join(mod_new).rstrip()+"\n")
os.rename(P,f"{D}/mod.rs")
print("slack dir split done; mod.rs:",len(mod_new),"lines")
