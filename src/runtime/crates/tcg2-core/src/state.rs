//! Save states: the whole machine as bytes.
//!
//! The kernel keeps nothing of the game outside the machine (its dispatcher
//! resumes at `cpu.pc`, with no native stack of the game's), so a machine's
//! fields are the game's entire state. The format is a fixed sequence of
//! little-endian fields after a magic and version; a state loads only into
//! a kernel of the same version, for the same ROM.

const MAGIC: &[u8; 8] = b"TCG2STAT";
const VERSION: u32 = 1;

#[derive(Default)]
pub struct Writer {
    pub bytes: Vec<u8>,
}

impl Writer {
    pub fn u8(&mut self, v: u8) {
        self.bytes.push(v);
    }
    pub fn bool(&mut self, v: bool) {
        self.bytes.push(v as u8);
    }
    pub fn u16(&mut self, v: u16) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u32(&mut self, v: u32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn u64(&mut self, v: u64) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn f32(&mut self, v: f32) {
        self.bytes.extend_from_slice(&v.to_le_bytes());
    }
    pub fn bytes(&mut self, v: &[u8]) {
        self.u32(v.len() as u32);
        self.bytes.extend_from_slice(v);
    }
}

pub struct Reader<'a> {
    bytes: &'a [u8],
    at: usize,
}

#[derive(Debug, PartialEq, Eq)]
pub struct StateError(pub String);

impl std::fmt::Display for StateError {
    fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
        write!(f, "bad save state: {}", self.0)
    }
}

impl std::error::Error for StateError {}

impl<'a> Reader<'a> {
    fn take(&mut self, n: usize) -> Result<&'a [u8], StateError> {
        let end = self.at.checked_add(n).filter(|&e| e <= self.bytes.len());
        let end = end.ok_or_else(|| StateError("truncated".into()))?;
        let slice = &self.bytes[self.at..end];
        self.at = end;
        Ok(slice)
    }
    pub fn u8(&mut self) -> Result<u8, StateError> {
        Ok(self.take(1)?[0])
    }
    pub fn bool(&mut self) -> Result<bool, StateError> {
        Ok(self.u8()? != 0)
    }
    pub fn u16(&mut self) -> Result<u16, StateError> {
        Ok(u16::from_le_bytes(self.take(2)?.try_into().unwrap()))
    }
    pub fn u32(&mut self) -> Result<u32, StateError> {
        Ok(u32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    pub fn u64(&mut self) -> Result<u64, StateError> {
        Ok(u64::from_le_bytes(self.take(8)?.try_into().unwrap()))
    }
    pub fn f32(&mut self) -> Result<f32, StateError> {
        Ok(f32::from_le_bytes(self.take(4)?.try_into().unwrap()))
    }
    /// A length-prefixed block of exactly `into.len()` bytes.
    pub fn fill(&mut self, into: &mut [u8]) -> Result<(), StateError> {
        let n = self.u32()? as usize;
        if n != into.len() {
            return Err(StateError(format!(
                "a block of {n} bytes where {} belong",
                into.len()
            )));
        }
        into.copy_from_slice(self.take(n)?);
        Ok(())
    }
    /// A length-prefixed block of any length.
    pub fn vec(&mut self) -> Result<Vec<u8>, StateError> {
        let n = self.u32()? as usize;
        Ok(self.take(n)?.to_vec())
    }
}

pub fn header(w: &mut Writer) {
    w.bytes.extend_from_slice(MAGIC);
    w.u32(VERSION);
}

pub fn open(bytes: &[u8]) -> Result<Reader<'_>, StateError> {
    let mut r = Reader { bytes, at: 0 };
    if r.take(8)? != MAGIC {
        return Err(StateError("not a save state".into()));
    }
    let version = r.u32()?;
    if version != VERSION {
        return Err(StateError(format!(
            "version {version}, this build reads {VERSION}"
        )));
    }
    Ok(r)
}
