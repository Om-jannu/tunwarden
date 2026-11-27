import sys
p="crates/tunwarden-core/src/config/edit.rs"; s=open(p).read()
pairs=[
('#[error("an entry named \\"{0}\\" already exists")]\n    Exists(String),','#[error("a {0} named \\"{1}\\" already exists")]\n    Exists(&\'static str, String),'),
("        return Err(EditError::Exists(name.to_owned()));\n    }\n    let mut table = Table::new();","        return Err(EditError::Exists(\"tunnel\", name.to_owned()));\n    }\n    let mut table = Table::new();"),
("        return Err(EditError::Exists(name.to_owned()));\n    }\n    let mut entry = Table::new();","        let what = if table == \"logins\" { \"login\" } else { \"profile\" };\n        return Err(EditError::Exists(what, name.to_owned()));\n    }\n    let mut entry = Table::new();"),
('Err(EditError::Exists("db".into()))','Err(EditError::Exists("tunnel", "db".into()))'),
]
for a,b in pairs:
    if a not in s: sys.exit("nf: "+a[:70])
    s=s.replace(a,b,1)
open(p,"w").write(s); print("ok")
