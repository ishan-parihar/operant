import re,os
P="crates/operant-channels/src/telegram.rs"
src=open(P).read(); lines=src.split("\n")
def L(n): return lines[n-1]
for n,a in {16:"const TELEGRAM_MAX_MESSAGE_LENGTH",544:"pub struct TelegramChannel {",
            636:"impl TelegramChannel {",3276:"impl Channel for TelegramChannel {",
            4359:"mod tests"}.items():
    assert a in L(n),(n,L(n)[:60])
def ext(s):
    j=s
    while j>1:
        st=L(j-1).strip()
        if st=="" or not(st.endswith(";") or st.endswith("}") or st.endswith("{")):
            j-=1; continue
        break
    return j
h_ext,a_ext,i_ext,t_ext,x_ext,tests_ext=ext(16),ext(544),ext(636),ext(3276),ext(4257),ext(4359)
uses=[];i=0
while i<16:
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
            # anything else at col0 is string-literal content: leave mode intact
            if not ln.startswith("pub") and re.match(r'^(async fn |fn |struct |enum |const |static |type )',ln):
                out.append("pub(crate) "+ln);continue
        else:
            if mode=='struct' and re.match(r'^    ([a-z_][a-z0-9_]*)\s*:',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
            if mode=='impl_inherent' and re.match(r'^    (?=(?:async\s+)?fn\b)',ln) and not ln.startswith("    pub"):
                out.append("    pub(crate) "+ln[4:]);continue
        out.append(ln)
    return "\n".join(out)

D="crates/operant-channels/src/telegram"
os.makedirs(D,exist_ok=True)
open(f"{D}/helpers.rs","w").write(
 "//! Helpers, tunables, attachment parsing, and poll-recovery state extracted\n//! verbatim from the former telegram.rs monolith.\n\n"
 +USES+"\nuse super::*;\n\n"+widen("\n".join(lines[h_ext-1:a_ext-1]).rstrip())+"\n")
open(f"{D}/channel_impl.rs","w").write(
 "//! Inherent methods on `TelegramChannel` extracted verbatim.\n\n"
 +USES+"\nuse super::*;\n\n"+widen("\n".join(lines[i_ext-1:t_ext-1]).rstrip()).replace("super::transcription::","crate::transcription::")+"\n")
body_t=lines[tests_ext:]
_mti=next(i for i,l in enumerate(body_t) if l.strip()=="mod tests {")
body_t=body_t[_mti+1:]  # skip 'mod tests {'
while body_t and body_t[-1].strip()=="": body_t.pop()
assert body_t[-1].strip()=="}"
open(f"{D}/tests.rs","w").write(
 "//! Telegram channel tests (verbatim body of the former inline `mod tests`).\n"+"\n".join(body_t[:-1]).rstrip()+"\n")
mod_new=(lines[:h_ext-1]
 +["// Split modules (dedup pass 6) - paths unchanged.","mod channel_impl;","mod helpers;",
   "#[cfg(test)]","mod tests;",""]
 +["pub(crate) use helpers::*;",""]
 +lines[a_ext-1:i_ext-1]      # struct + EditMessageResult
 +lines[t_ext-1:tests_ext-1]) # Channel trait impl + Drop + poll loops
open(P,"w").write("\n".join(mod_new).rstrip()+"\n")
os.rename(P,f"{D}/mod.rs")
print("telegram dir split done; mod.rs:",len(mod_new),"lines")
