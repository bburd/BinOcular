use serde::{Deserialize, Serialize};

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct Schema {
    pub schema_name: String,
    pub schema_version: u32,
    pub endianness: Option<Endianness>,
    #[serde(default)]
    pub structures: Vec<StructureDef>,
    pub fields: Vec<FieldItem>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum Endianness {
    Little,
    Big,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum IntExprOp {
    Add,
    Sub,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum IntExpr {
    Const {
        #[serde(rename = "const")]
        value: i64,
    },
    FieldRef {
        field: String,
    },
    Binary {
        op: IntExprOp,
        left: Box<IntExpr>,
        right: Box<IntExpr>,
    },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(tag = "kind", content = "value")]
pub enum OffsetKind {
    Absolute(u64),
    Relative(u64),
    FieldRef(String),
    Expr(IntExpr),
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum LengthSpec {
    Literal(u64),
    FieldRef { field: String },
    Expr { expr: IntExpr },
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(rename_all = "lowercase")]
pub enum FieldType {
    U8,
    U16,
    U32,
    U64,
    I32,
    F32,
    Bytes,
    Ascii,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct RepeatInfo {
    pub count: u64,
    #[serde(default)]
    pub stride: Option<u64>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct FieldDef {
    pub name: String,
    #[serde(rename = "type")]
    pub ty: FieldType,
    pub offset: OffsetKind,
    pub length: Option<LengthSpec>,
    pub endianness: Option<Endianness>,
    pub description: Option<String>,
    pub repeat: Option<RepeatInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructureDef {
    pub name: String,
    pub fields: Vec<FieldDef>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
pub struct StructInstanceDef {
    pub name: String,
    #[serde(rename = "struct")]
    pub struct_name: String,
    pub offset: OffsetKind,
    pub description: Option<String>,
    pub repeat: Option<RepeatInfo>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldItem {
    Field(FieldDef),
    StructInstance(StructInstanceDef),
}
