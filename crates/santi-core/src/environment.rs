use std::{
    collections::{BTreeMap, HashSet},
    process::Command,
};

pub const RESERVED: &str = "SANTI_";

const ALLOWED: &[&str] = &[
    "HOME",
    "USER",
    "LOGNAME",
    "PATH",
    "LANG",
    "LC_ALL",
    "TERM",
    "TMPDIR",
    "XDG_RUNTIME_DIR",
    "DBUS_SESSION_BUS_ADDRESS",
    "SHELL",
    "APPDATA",
    "COMSPEC",
    "HOMEDRIVE",
    "HOMEPATH",
    "LOCALAPPDATA",
    "OS",
    "PATHEXT",
    "PROGRAMDATA",
    "PROGRAMFILES",
    "PROGRAMFILES(X86)",
    "PROGRAMW6432",
    "SYSTEMDRIVE",
    "SYSTEMROOT",
    "TEMP",
    "TMP",
    "USERDOMAIN",
    "USERNAME",
    "USERPROFILE",
    "WINDIR",
];

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Declaration {
    pub scope: String,
    pub name: String,
    pub value: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Unresolved {
    pub scope: String,
    pub name: String,
    pub reference: String,
}

impl Unresolved {
    pub fn dedupe(&self, strand: &str) -> String {
        format!(
            "env_unresolved:{}:{}:{}:{}",
            strand, self.scope, self.name, self.reference
        )
    }
}

#[derive(Debug, Clone, Default)]
pub struct Resolved {
    pub values: BTreeMap<String, String>,
    pub unresolved: Vec<Unresolved>,
}

pub fn resolve(
    declared: impl IntoIterator<Item = Declaration>,
    lookup: &dyn Fn(&str) -> Option<String>,
) -> Resolved {
    let mut held = Resolved::default();
    for declaration in declared {
        if declaration.name.starts_with(RESERVED) {
            continue;
        }
        let Some(reference) = declaration.value.strip_prefix("env://") else {
            held.values.insert(declaration.name, declaration.value);
            continue;
        };
        let reference = reference.trim().to_string();
        match lookup(&reference).filter(|found| !found.is_empty()) {
            Some(found) => {
                held.values.insert(declaration.name, found);
            }
            None => {
                held.values
                    .insert(declaration.name.clone(), declaration.value);
                held.unresolved.push(Unresolved {
                    scope: declaration.scope,
                    name: declaration.name,
                    reference,
                });
            }
        }
    }
    held
}

pub fn validate(declared: &BTreeMap<String, String>) -> Result<(), String> {
    for name in declared.keys() {
        legal(name)?;
    }
    Ok(())
}

pub fn legal(name: &str) -> Result<(), String> {
    let mut characters = name.chars();
    let first = characters
        .next()
        .ok_or_else(|| "environment name must not be empty".to_string())?;
    if !(first == '_' || first.is_ascii_alphabetic())
        || !characters.all(|character| character == '_' || character.is_ascii_alphanumeric())
    {
        return Err(format!(
            "environment name {name:?} must be a portable shell identifier"
        ));
    }
    if name.starts_with(RESERVED) {
        return Err(format!(
            "environment name {name} uses the reserved {RESERVED} prefix"
        ));
    }
    Ok(())
}

pub fn allow(command: &mut Command) {
    for name in ALLOWED {
        if let Some(value) = std::env::var_os(name) {
            command.env(name, value);
        }
    }
}

pub fn allowed() -> HashSet<&'static str> {
    ALLOWED.iter().copied().collect()
}
