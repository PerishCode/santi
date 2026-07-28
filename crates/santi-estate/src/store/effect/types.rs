#[derive(Clone, Copy)]
pub struct EffectDraft<'a> {
    pub tag: &'a str,
    pub turn: &'a str,
    pub call: Option<&'a str>,
    pub kind: &'a str,
    pub metadata: Option<&'a serde_json::Value>,
    pub created: &'a str,
}

pub struct RedemptionDraft<'a> {
    pub result: &'a str,
    pub call: &'a str,
    pub output: Option<&'a serde_json::Value>,
    pub error: Option<&'a str>,
    pub outcome: santi_model::effect::Outcome,
    pub occurred: &'a str,
}
