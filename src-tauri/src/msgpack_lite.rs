//! Mini-décodeur MessagePack maison, partagé entre `controller_assignments.rs` (assignations
//! Command Center) et `preset_chain_params.rs` (état actif/inactif de slot) — mêmes tags observés
//! dans le dump preset `ed:03`, pas de dépendance externe (`rmp`/`rmpv`), esprit hand-rolled déjà
//! établi dans ce projet (`preset_chain_params::read_params_hex`).

#[derive(Debug, Clone)]
pub enum Value {
    Int(i64),
    Bool(bool),
    Float(f32),
    Str(String),
    /// Contenu ignoré (jamais consulté par les décodeurs actuels) : seul le bon avancement du
    /// curseur importe pour pouvoir sauter par-dessus un tableau imbriqué non pertinent.
    Array,
    Map(Vec<(u8, Value)>),
    Nil,
}

impl Value {
    pub fn as_map(&self) -> Option<&[(u8, Value)]> {
        match self {
            Value::Map(m) => Some(m),
            _ => None,
        }
    }
    pub fn as_int(&self) -> Option<i64> {
        match self {
            Value::Int(i) => Some(*i),
            _ => None,
        }
    }
    pub fn as_bool(&self) -> Option<bool> {
        match self {
            Value::Bool(b) => Some(*b),
            _ => None,
        }
    }
    pub fn as_float(&self) -> Option<f32> {
        match self {
            Value::Float(f) => Some(*f),
            _ => None,
        }
    }
    pub fn as_str(&self) -> Option<&str> {
        match self {
            Value::Str(s) => Some(s.as_str()),
            _ => None,
        }
    }
}

pub fn map_get<'a>(pairs: &'a [(u8, Value)], key: u8) -> Option<&'a Value> {
    pairs.iter().find(|(k, _)| *k == key).map(|(_, v)| v)
}

/// Décode une valeur MessagePack à partir de `data[*pos]`, avance `*pos`. Tags reconnus : positive
/// et négative fixint, fixmap, fixarray, fixstr, nil, bool, float32, uint8/16/32 — c'est tout ce
/// qu'on observe dans ces sous-formats ; tout autre tag fait échouer le parsing (pas de faux
/// positif silencieux, pas de valeur par défaut devinée).
pub fn parse_value(data: &[u8], pos: &mut usize) -> Option<Value> {
    let tag = *data.get(*pos)?;
    *pos += 1;
    match tag {
        0x00..=0x7f => Some(Value::Int(tag as i64)),
        0xe0..=0xff => Some(Value::Int(tag as i8 as i64)),
        0x80..=0x8f => {
            let n = (tag & 0x0f) as usize;
            let mut pairs = Vec::with_capacity(n);
            for _ in 0..n {
                let key = match parse_value(data, pos)? {
                    Value::Int(i) if (0..=255).contains(&i) => i as u8,
                    _ => return None,
                };
                let val = parse_value(data, pos)?;
                pairs.push((key, val));
            }
            Some(Value::Map(pairs))
        }
        0x90..=0x9f => {
            let n = (tag & 0x0f) as usize;
            for _ in 0..n {
                parse_value(data, pos)?;
            }
            Some(Value::Array)
        }
        0xa0..=0xbf => {
            let len = (tag & 0x1f) as usize;
            let bytes = data.get(*pos..*pos + len)?;
            *pos += len;
            let s = String::from_utf8_lossy(bytes)
                .trim_end_matches('\0')
                .to_string();
            Some(Value::Str(s))
        }
        0xc0 => Some(Value::Nil),
        0xc2 => Some(Value::Bool(false)),
        0xc3 => Some(Value::Bool(true)),
        0xca => {
            let bytes = data.get(*pos..*pos + 4)?;
            *pos += 4;
            Some(Value::Float(f32::from_be_bytes(bytes.try_into().ok()?)))
        }
        0xcc => {
            let v = *data.get(*pos)?;
            *pos += 1;
            Some(Value::Int(v as i64))
        }
        0xcd => {
            let bytes = data.get(*pos..*pos + 2)?;
            *pos += 2;
            Some(Value::Int(u16::from_be_bytes(bytes.try_into().ok()?) as i64))
        }
        0xce => {
            let bytes = data.get(*pos..*pos + 4)?;
            *pos += 4;
            Some(Value::Int(u32::from_be_bytes(bytes.try_into().ok()?) as i64))
        }
        _ => None,
    }
}
