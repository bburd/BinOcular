use serde::de::{self, MapAccess, Visitor};
use serde::ser::SerializeMap;
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use std::fmt;

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

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WhenCondition {
    pub field: String,
    pub op: WhenOp,
}

#[derive(Debug, Clone, PartialEq, Eq, Serialize)]
#[serde(rename_all = "snake_case")]
pub enum WhenOp {
    Equals(i128),
    NotEquals(i128),
    BitSet(u8),
}

impl<'de> Deserialize<'de> for WhenCondition {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        deserializer.deserialize_map(WhenConditionVisitor)
    }
}

impl Serialize for WhenCondition {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        let mut map = serializer.serialize_map(Some(2))?;
        map.serialize_entry("field", &self.field)?;
        match self.op {
            WhenOp::Equals(value) => map.serialize_entry("equals", &value)?,
            WhenOp::NotEquals(value) => map.serialize_entry("not_equals", &value)?,
            WhenOp::BitSet(bit) => map.serialize_entry("bit_set", &bit)?,
        }
        map.end()
    }
}

struct WhenConditionVisitor;

impl<'de> Visitor<'de> for WhenConditionVisitor {
    type Value = WhenCondition;

    fn expecting(&self, formatter: &mut fmt::Formatter) -> fmt::Result {
        formatter.write_str("a when condition mapping")
    }

    fn visit_map<M>(self, mut map: M) -> Result<Self::Value, M::Error>
    where
        M: MapAccess<'de>,
    {
        let mut field = None;
        let mut op = None;

        while let Some(key) = map.next_key::<String>()? {
            match key.as_str() {
                "field" => {
                    if field.is_some() {
                        return Err(de::Error::duplicate_field("field"));
                    }
                    field = Some(map.next_value()?);
                }
                "equals" => {
                    set_when_op(&mut op, WhenOp::Equals(map.next_value()?), "equals")?;
                }
                "not_equals" => {
                    set_when_op(&mut op, WhenOp::NotEquals(map.next_value()?), "not_equals")?;
                }
                "bit_set" => {
                    let bit: u64 = map.next_value()?;
                    let bit = u8::try_from(bit).map_err(|_| {
                        de::Error::custom("when bit_set index must be between 0 and 63")
                    })?;
                    set_when_op(&mut op, WhenOp::BitSet(bit), "bit_set")?;
                }
                other => {
                    return Err(de::Error::unknown_field(
                        other,
                        &["field", "equals", "not_equals", "bit_set"],
                    ));
                }
            }
        }

        let field = field.ok_or_else(|| de::Error::missing_field("field"))?;
        let op = op.ok_or_else(|| {
            de::Error::custom("when condition must specify exactly one condition operator")
        })?;

        Ok(WhenCondition { field, op })
    }
}

fn set_when_op<E>(slot: &mut Option<WhenOp>, value: WhenOp, name: &'static str) -> Result<(), E>
where
    E: de::Error,
{
    if slot.is_some() {
        return Err(E::custom(format!(
            "when condition cannot specify multiple operators; found `{name}` after another operator"
        )));
    }

    *slot = Some(value);
    Ok(())
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
    pub when: Option<WhenCondition>,
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
    pub when: Option<WhenCondition>,
}

#[derive(Debug, Clone, Serialize, Deserialize)]
#[serde(untagged)]
pub enum FieldItem {
    Field(FieldDef),
    StructInstance(StructInstanceDef),
}
