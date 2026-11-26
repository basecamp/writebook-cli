use serde::{Deserialize, Serialize, Serializer};

/// What a leaf of a book is, as Writebook names it in a leaf's `type`. A kind
/// this version doesn't know keeps its name, so it's written back unchanged.
#[derive(Deserialize, Clone, Debug, PartialEq, Eq)]
#[serde(from = "String")]
pub enum LeafKind {
    Page,
    Section,
    Picture,
    Unknown(String),
}

impl LeafKind {
    pub fn name(&self) -> &str {
        match self {
            LeafKind::Page => "Page",
            LeafKind::Section => "Section",
            LeafKind::Picture => "Picture",
            LeafKind::Unknown(name) => name,
        }
    }
}

impl From<String> for LeafKind {
    fn from(name: String) -> LeafKind {
        match name.as_str() {
            "Page" => LeafKind::Page,
            "Section" => LeafKind::Section,
            "Picture" => LeafKind::Picture,
            _ => LeafKind::Unknown(name),
        }
    }
}

impl Serialize for LeafKind {
    fn serialize<S: Serializer>(&self, serializer: S) -> Result<S::Ok, S::Error> {
        serializer.serialize_str(self.name())
    }
}
