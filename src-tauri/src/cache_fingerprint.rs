//! Bounded change-detection samples. Digests are local indexes, not encryption.
use sha2::{Digest, Sha256};

#[derive(Clone, Debug, Default, PartialEq, Eq, serde::Serialize, serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct SampleFingerprint {
    pub sampled_len: usize,
    pub sha256: String,
}
impl SampleFingerprint {
    pub fn of(bytes: &[u8]) -> Self {
        Self {
            sampled_len: bytes.len(),
            sha256: format!("{:x}", Sha256::digest(bytes)),
        }
    }
    pub fn valid(&self, cap: usize) -> bool {
        self.sampled_len > 0
            && self.sampled_len <= cap
            && self.sha256.len() == 64
            && self
                .sha256
                .bytes()
                .all(|b| b.is_ascii_digit() || (b'a'..=b'f').contains(&b))
    }
    pub fn is_empty(&self) -> bool {
        self.sampled_len == 0
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn digest_and_length_are_exact_and_malformed_marks_fail_closed() {
        let fp = SampleFingerprint::of(b"abc");
        assert_eq!(fp.sampled_len, 3);
        assert_eq!(
            fp.sha256,
            "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad"
        );
        assert!(fp.valid(3));
        assert!(!fp.valid(2));
        assert!(!SampleFingerprint {
            sampled_len: 3,
            sha256: "raw abc".into()
        }
        .valid(3));
        assert!(!SampleFingerprint::default().valid(3));
        assert!(serde_json::from_str::<SampleFingerprint>("[97,98,99]").is_err());
    }
}
