//! The YAML spelling of types: a string in the textual type syntax (`list<Order>`,
//! `optional<string>`, `decimal`, `Card | Cash`), a mapping of field to type for a record, or
//! `{ oneOf: { Tag: payload-or-null, ... } }` for a tagged union.

use vertep_core::types::{Field, Type, Variant};

use crate::node::{Kind, Node};

/// Why a node is not a type.
pub(crate) struct TypeNodeError {
    /// The offending node's position is `node.pos` plus, for syntax errors, this offset in
    /// the scalar.
    pub node: Node,
    pub offset: Option<usize>,
    pub message: String,
}

/// Read a type spelled in YAML.
pub(crate) fn type_from_node(v: &Node) -> Result<Type, Vec<TypeNodeError>> {
    match &v.kind {
        _ if v.is_null() => Err(vec![TypeNodeError {
            node: v.clone(),
            offset: None,
            message: "expected a type, found null".into(),
        }]),
        Kind::Scalar { text, .. } => vertep_core::types::parse_type(text).map_err(|e| {
            vec![TypeNodeError {
                node: v.clone(),
                offset: Some(e.offset),
                message: format!("bad type `{text}`: {}", e.message),
            }]
        }),
        Kind::Seq(_) => Err(vec![TypeNodeError {
            node: v.clone(),
            offset: None,
            message: "expected a type (a string, a record mapping or `oneOf`), found a list".into(),
        }]),
        Kind::Map(entries) => {
            let mut errors = Vec::new();
            if let [(k, variants)] = entries.as_slice()
                && k.scalar() == Some("oneOf")
            {
                let Some(vs) = variants.as_map() else {
                    return Err(vec![TypeNodeError {
                        node: variants.clone(),
                        offset: None,
                        message: "`oneOf` takes a mapping of tag to payload type or null".into(),
                    }]);
                };
                let mut out = Vec::new();
                for (tag, payload) in vs {
                    let payload = if payload.is_null() {
                        None
                    } else {
                        match type_from_node(payload) {
                            Ok(t) => Some(t),
                            Err(e) => {
                                errors.extend(e);
                                None
                            }
                        }
                    };
                    out.push(Variant::new(tag.scalar().unwrap_or_default(), payload));
                }
                return if errors.is_empty() {
                    Ok(Type::Union(out))
                } else {
                    Err(errors)
                };
            }
            let mut fields = Vec::new();
            for (k, t) in entries {
                match type_from_node(t) {
                    Ok(ty) => fields.push(Field::new(k.scalar().unwrap_or_default(), ty)),
                    Err(e) => errors.extend(e),
                }
            }
            if errors.is_empty() {
                Ok(Type::Record(fields))
            } else {
                Err(errors)
            }
        }
    }
}

fn unique<'a>(names: impl Iterator<Item = &'a str>) -> bool {
    let mut seen = std::collections::HashSet::new();
    names.into_iter().all(|n| seen.insert(n))
}

/// Spell a type in YAML: records as mappings and unions as `oneOf` mappings when their names
/// are unique (and a record is not a lone `oneOf` field), everything else as a string.
pub(crate) fn type_to_node(t: &Type) -> Node {
    match t {
        Type::Record(fields)
            if unique(fields.iter().map(|f| f.name.as_str()))
                && !(fields.len() == 1 && fields[0].name == "oneOf") =>
        {
            Node::map_owned(
                fields
                    .iter()
                    .map(|f| (f.name.clone(), type_to_node(&f.ty)))
                    .collect(),
            )
        }
        Type::Union(vs) if !vs.is_empty() && unique(vs.iter().map(|v| v.tag.as_str())) => {
            Node::map(vec![(
                "oneOf",
                Node::map_owned(
                    vs.iter()
                        .map(|v| {
                            let p = v.payload.as_ref().map_or_else(Node::null, type_to_node);
                            (v.tag.clone(), p)
                        })
                        .collect(),
                ),
            )])
        }
        other => Node::str(other.to_string()),
    }
}
