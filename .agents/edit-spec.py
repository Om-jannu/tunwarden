import sys
p="crates/tunwarden-core/src/supervisor/mod.rs"; s=open(p).read()
pairs=[
("use crate::config::{Project, ToolPaths, TunnelConfig};","use crate::config::{Config, ToolPaths, TunnelConfig};"),
("    pub fn new(tunnel: &TunnelConfig, project: Option<&Project>, tools: &ToolPaths) -> Spec {",
 "    /// The spec of `tunnel`, with its profile's environment and its login's patterns.\n    pub fn new(tunnel: &TunnelConfig, config: &Config) -> Spec {\n        let (profile, login) = config.context_of(tunnel);"),
("            .output_patterns()\n","            .output_patterns(login)\n"),
("                let hint = tunnel.render_hint(p.hint);","                let hint = tunnel.render_hint(&p.hint);"),
("            plan: tunnel.launch_plan(project),\n            tools: tools.clone(),","            plan: tunnel.launch_plan(profile),\n            tools: config.tools.clone(),"),
]
for a,b in pairs:
    if a not in s: sys.exit("nf: "+a[:60])
    s=s.replace(a,b,1)
import re
s,n=re.subn(r"TunnelProcess::spawn_clean\(\s*([^,]+),\s*&spec\.plan\.args,\s*&spec\.plan\.env,\s*&spec\.plan\.env_remove,\s*\)", r"TunnelProcess::spawn(\1, &spec.plan.args, &spec.plan.env)", s)
if n!=1: sys.exit(f"spawn_clean {n}")
open(p,"w").write(s); print("ok")
