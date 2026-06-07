use std::fmt;

const BLOB_HEX_LEN: usize = 64;

#[derive(Debug, Clone, Copy, PartialEq, Eq, Hash)]
pub enum BlobKind {
    Eml,
    Att,
}

impl BlobKind {
    pub fn suffix(self) -> &'static str {
        match self {
            Self::Eml => "eml",
            Self::Att => "att",
        }
    }

    pub fn from_suffix(suffix: &str) -> Option<Self> {
        match suffix {
            "eml" => Some(Self::Eml),
            "att" => Some(Self::Att),
            _ => None,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq, Hash)]
pub struct BlobId {
    hex: String,
    kind: BlobKind,
}
impl BlobId {
    pub fn new(hex: impl Into<String>, kind: BlobKind) -> Result<Self, BlobIdParseError> {
        let hex = hex.into();
        validate_hex(&hex)?;
        Ok(Self { hex, kind })
    }

    pub fn parse(value: &str) -> Result<Self, BlobIdParseError> {
        let (hex, suffix) = value
            .split_once('.')
            .ok_or_else(|| BlobIdParseError(value.to_owned()))?;
        let kind =
            BlobKind::from_suffix(suffix).ok_or_else(|| BlobIdParseError(value.to_owned()))?;
        Self::new(hex, kind)
    }

    pub fn hex(&self) -> &str {
        &self.hex
    }

    pub fn kind(&self) -> BlobKind {
        self.kind
    }

    pub fn file_name(&self) -> String {
        format!("{}.{}.zst", self.hex, self.kind.suffix())
    }
}

impl fmt::Display for BlobId {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{}.{}", self.hex, self.kind.suffix())
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BlobIdParseError(String);

impl fmt::Display for BlobIdParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "invalid blob id: {}", self.0)
    }
}

impl std::error::Error for BlobIdParseError {}

fn validate_hex(hex: &str) -> Result<(), BlobIdParseError> {
    if hex.len() != BLOB_HEX_LEN || !hex.as_bytes().iter().all(u8::is_ascii_hexdigit) {
        return Err(BlobIdParseError(hex.to_owned()));
    }
    Ok(())
}
