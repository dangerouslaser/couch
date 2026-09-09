//! Bounded OPACK subset used by Companion. Encoders emit literals; decoders
//! accept the scalar back-references emitted by Apple devices.
use crate::{Error, Result};
#[derive(Debug, Clone, PartialEq)]
pub enum Value {
    Null,
    Bool(bool),
    Int(u64),
    Float(f64),
    String(String),
    Data(Vec<u8>),
    Uuid([u8; 16]),
    Array(Vec<Value>),
    Dict(Vec<(String, Value)>),
}
impl From<&str> for Value {
    fn from(v: &str) -> Self {
        Self::String(v.into())
    }
}
impl From<u64> for Value {
    fn from(v: u64) -> Self {
        Self::Int(v)
    }
}
impl Value {
    pub fn dict(values: impl IntoIterator<Item = (&'static str, Value)>) -> Self {
        Self::Dict(values.into_iter().map(|(k, v)| (k.into(), v)).collect())
    }
    pub fn get(&self, key: &str) -> Option<&Value> {
        if let Self::Dict(v) = self {
            v.iter().find(|(k, _)| k == key).map(|(_, v)| v)
        } else {
            None
        }
    }
    pub fn uint(&self) -> Result<u64> {
        if let Self::Int(v) = self {
            Ok(*v)
        } else {
            Err(Error::Protocol)
        }
    }
    pub fn data(&self) -> Result<&[u8]> {
        if let Self::Data(v) = self {
            Ok(v)
        } else {
            Err(Error::Protocol)
        }
    }
    pub fn string(&self) -> Result<&str> {
        if let Self::String(v) = self {
            Ok(v)
        } else {
            Err(Error::Protocol)
        }
    }
}
pub fn encode(value: &Value) -> Result<Vec<u8>> {
    let mut out = Vec::new();
    write(value, &mut out, 0)?;
    if out.len() > 1_048_576 {
        return Err(Error::Protocol);
    }
    Ok(out)
}
fn blob(tag: u8, v: &[u8], out: &mut Vec<u8>) -> Result<()> {
    if v.len() > 1_048_576 {
        return Err(Error::Protocol);
    }
    if v.len() <= 32 {
        out.push(tag + v.len() as u8)
    } else if v.len() < 256 {
        out.extend([tag + 33, v.len() as u8]);
    } else {
        out.push(tag + if tag == 0x40 { 36 } else { 35 });
        out.extend((v.len() as u32).to_le_bytes());
    }
    out.extend(v);
    Ok(())
}
fn write(v: &Value, out: &mut Vec<u8>, depth: usize) -> Result<()> {
    if depth > 32 || out.len() > 1_048_576 {
        return Err(Error::Protocol);
    }
    match v {
        Value::Null => out.push(4),
        Value::Bool(v) => out.push(if *v { 1 } else { 2 }),
        Value::Int(v) => {
            if *v < 40 {
                out.push(*v as u8 + 8)
            } else {
                let n: usize = if *v <= 255 {
                    1
                } else if *v <= 65535 {
                    2
                } else if *v <= u32::MAX as u64 {
                    4
                } else {
                    8
                };
                out.push(0x30 + n.ilog2() as u8);
                out.extend(&v.to_le_bytes()[..n]);
            }
        }
        Value::Float(v) => {
            out.push(0x36);
            out.extend(v.to_le_bytes())
        }
        Value::String(v) => blob(0x40, v.as_bytes(), out)?,
        Value::Data(v) => blob(0x70, v, out)?,
        Value::Uuid(v) => {
            out.push(5);
            out.extend(v);
        }
        Value::Array(v) => {
            if v.len() > 4096 {
                return Err(Error::Protocol);
            }
            out.push(0xd0 + v.len().min(15) as u8);
            for x in v {
                write(x, out, depth + 1)?;
            }
            if v.len() >= 15 {
                out.push(3)
            }
        }
        Value::Dict(v) => {
            if v.len() > 4096 {
                return Err(Error::Protocol);
            }
            out.push(0xe0 + v.len().min(15) as u8);
            for (k, x) in v {
                write(&Value::String(k.clone()), out, depth + 1)?;
                write(x, out, depth + 1)?;
            }
            if v.len() >= 15 {
                out.push(3)
            }
        }
    }
    Ok(())
}
pub fn decode(data: &[u8]) -> Result<Value> {
    if data.len() > 1_048_576 {
        return Err(Error::Protocol);
    }
    let mut p = Parser {
        data,
        pos: 0,
        refs: vec![],
        nodes: 4096,
        budget: 4_194_304,
    };
    let value = p.value(0)?;
    if p.pos != data.len() {
        return Err(Error::Protocol);
    }
    Ok(value)
}
struct Parser<'a> {
    data: &'a [u8],
    pos: usize,
    refs: Vec<Value>,
    nodes: usize,
    budget: usize,
}
impl Parser<'_> {
    fn take(&mut self, n: usize) -> Result<&[u8]> {
        let end = self
            .pos
            .checked_add(n)
            .filter(|e| *e <= self.data.len())
            .ok_or(Error::Protocol)?;
        let data = &self.data[self.pos..end];
        self.pos = end;
        Ok(data)
    }
    fn number(&mut self, n: usize) -> Result<u64> {
        if n > 8 {
            return Err(Error::Protocol);
        }
        let mut out = [0; 8];
        out[..n].copy_from_slice(self.take(n)?);
        Ok(u64::from_le_bytes(out))
    }
    fn value(&mut self, depth: usize) -> Result<Value> {
        if depth > 32 || self.nodes == 0 {
            return Err(Error::Protocol);
        }
        self.nodes -= 1;
        let tag = self.number(1)? as u8;
        let mut reference = true;
        let value = match tag {
            1 | 2 => {
                reference = false;
                Value::Bool(tag == 1)
            }
            4 => {
                reference = false;
                Value::Null
            }
            5 => Value::Uuid(self.take(16)?.try_into().unwrap()),
            6 => Value::Int(self.number(8)?),
            8..=0x2f => {
                reference = false;
                Value::Int((tag - 8) as u64)
            }
            0x30..=0x33 => Value::Int(self.number(1 << (tag - 0x30))?),
            0x35 => Value::Float(f32::from_le_bytes(self.take(4)?.try_into().unwrap()) as f64),
            0x36 => Value::Float(f64::from_le_bytes(self.take(8)?.try_into().unwrap())),
            0x40..=0x64 | 0x70..=0x94 => {
                let string = tag < 0x70;
                let base = if string { 0x40 } else { 0x70 };
                let n = if tag <= base + 32 {
                    (tag - base) as usize
                } else {
                    let width = if string {
                        tag - base - 32
                    } else {
                        1 << (tag - base - 33)
                    };
                    usize::try_from(self.number(width as usize)?).map_err(|_| Error::Protocol)?
                };
                if n > self.budget {
                    return Err(Error::Protocol);
                }
                self.budget -= n;
                let bytes = self.take(n)?;
                if string {
                    Value::String(
                        std::str::from_utf8(bytes)
                            .map_err(|_| Error::Protocol)?
                            .into(),
                    )
                } else {
                    Value::Data(bytes.into())
                }
            }
            0xa0..=0xc4 => {
                reference = false;
                let index = if tag <= 0xc0 {
                    (tag - 0xa0) as usize
                } else {
                    usize::try_from(self.number(1 << (tag - 0xc1))?).map_err(|_| Error::Protocol)?
                };
                let v = self.refs.get(index).ok_or(Error::Protocol)?;
                let cost = match v {
                    Value::String(s) => s.len(),
                    Value::Data(d) => d.len(),
                    _ => 16,
                };
                if cost > self.budget {
                    return Err(Error::Protocol);
                }
                self.budget -= cost;
                v.clone()
            }
            0xd0..=0xef => {
                reference = false;
                let mut values = vec![];
                let count = (tag & 15) as usize;
                loop {
                    if count < 15 && values.len() == count {
                        break;
                    }
                    if count == 15 && self.data.get(self.pos) == Some(&3) {
                        self.pos += 1;
                        break;
                    }
                    let a = self.value(depth + 1)?;
                    if tag >= 0xe0 {
                        let key = a.string()?.to_string();
                        if values.iter().any(|v: &(String, Value)| v.0 == key) {
                            return Err(Error::Protocol);
                        }
                        let b = self.value(depth + 1)?;
                        values.push((key, b));
                    } else {
                        values.push((String::new(), a));
                    }
                }
                if tag >= 0xe0 {
                    Value::Dict(values)
                } else {
                    Value::Array(values.into_iter().map(|(_, v)| v).collect())
                }
            }
            _ => return Err(Error::Protocol),
        };
        if reference && !self.refs.contains(&value) {
            self.refs.push(value.clone());
        }
        Ok(value)
    }
}
#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn literal_and_reference_vectors() {
        assert_eq!(
            decode(&[0xe1, 0x42, b'_', b't', 10]).unwrap(),
            Value::dict([("_t", 2.into())])
        );
        assert_eq!(
            decode(&[0xd2, 0x43, b'a', b'b', b'c', 0xa0]).unwrap(),
            Value::Array(vec!["abc".into(), "abc".into()])
        );
    }
    #[test]
    fn nested_pairing_payload_roundtrips() {
        let v = Value::dict([
            ("_pd", Value::Data(vec![6, 1, 1])),
            ("_pwTy", 1.into()),
            ("list", Value::Array((0..20).map(Value::Int).collect())),
        ]);
        assert_eq!(decode(&encode(&v).unwrap()).unwrap(), v);
    }
    #[test]
    fn malformed_references_lengths_and_depth_are_rejected() {
        for b in [
            vec![0xa0],
            vec![0x64, 255, 255, 255, 255],
            vec![0xe1],
            vec![0xd1; 40],
            vec![1, 2],
        ] {
            assert!(decode(&b).is_err())
        }
    }
}
