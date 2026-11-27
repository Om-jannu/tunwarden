import re,sys
p="crates/tunwarden-core/src/import.rs"; s=open(p).read()
s2,n=re.subn(r'json!\(\s*shlex::try_join\(words\.iter\(\)\.map\(String::as_str\)\)\s*\.unwrap_or\(command\.clone\(\)\)\s*\)',
             'json!(words.iter().map(|w| crate::types::quote(w)).collect::<Vec<_>>().join(" "))', s)
if n!=1: sys.exit(f"import {n}")
open(p,"w").write(s2)
p="crates/tunwarden-core/src/config/tests.rs"; s=open(p).read()
a='"6:8 profiles[1].name: profile name \\"P\\" is already used by profiles[0]"'
if a not in s: sys.exit("ctest")
open(p,"w").write(s.replace(a,'"5:8 profiles[1].name: profile name \\"P\\" is already used by profiles[0]"'))
print("ok")
