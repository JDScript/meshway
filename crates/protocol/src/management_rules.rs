use crate::object::wire_object;

#[derive(
    Debug, Clone, Copy, PartialEq, Eq, serde::Serialize, serde::Deserialize, utoipa::ToSchema,
)]
#[serde(rename_all = "camelCase", try_from = "String")]
pub enum NetworkNameTrim {
    UnicodeWhiteSpace,
}
impl TryFrom<String> for NetworkNameTrim {
    type Error = &'static str;
    fn try_from(value: String) -> Result<Self, Self::Error> {
        if value == "unicodeWhiteSpace" {
            Ok(Self::UnicodeWhiteSpace)
        } else {
            Err("unknown trim rule")
        }
    }
}

wire_object! {
    #[derive(Debug,Clone,Copy,PartialEq,Eq)]
    pub struct NetworkNameRules {
        pub min_codepoints: u16 [minimum=0,maximum=65535],
        pub max_codepoints: u16 [minimum=0,maximum=65535],
        pub max_input_utf8_bytes: u16 [minimum=0,maximum=65535],
        /// Unicode White_Space: U+0009..000D, 0020, 0085, 00A0, 1680,
        /// 2000..200A, 2028, 2029, 202F, 205F, 3000. U+FEFF is not trimmed.
        /// Raw input Cc rejection happens before trimming (including edge controls).
        pub trim: NetworkNameTrim,
        /// Reject every Unicode General_Category=Cc character in raw input.
        pub forbid_control_characters: bool,
    }
}
impl NetworkNameRules {
    pub const CURRENT: Self = Self {
        min_codepoints: 1,
        max_codepoints: 128,
        max_input_utf8_bytes: 1024,
        trim: NetworkNameTrim::UnicodeWhiteSpace,
        forbid_control_characters: true,
    };
}
