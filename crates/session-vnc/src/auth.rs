//! VNC Authentication (RFB security type 2): the server sends 16 random bytes, the client
//! returns them encrypted with DES, keyed by the password.
//!
//! The key is the first eight bytes of the password (shorter passwords are padded with zero
//! bytes, longer ones are silently truncated: that is how every VNC server works), with the
//! bits of each byte reversed. DES is weak by today's standards and the exchange does not
//! protect the rest of the session: VNC traffic is not encrypted.

use des::cipher::{BlockCipherEncrypt, KeyInit};
use des::Des;

/// The response to a 16-byte `challenge`.
pub fn response(challenge: &[u8; 16], password: &[u8]) -> [u8; 16] {
    let mut key = [0u8; 8];
    for (slot, byte) in key.iter_mut().zip(password) {
        *slot = byte.reverse_bits();
    }
    let cipher = Des::new(&key.into());
    let mut out = [0u8; 16];
    for (src, dst) in challenge.chunks_exact(8).zip(out.chunks_exact_mut(8)) {
        let mut block = [0u8; 8];
        block.copy_from_slice(src);
        let mut block = block.into();
        cipher.encrypt_block(&mut block);
        dst.copy_from_slice(&block);
    }
    out
}

#[cfg(test)]
mod tests {
    use super::*;

    fn hex(s: &str) -> Vec<u8> {
        (0..s.len())
            .step_by(2)
            .map(|i| u8::from_str_radix(&s[i..i + 2], 16).expect("hex"))
            .collect()
    }

    /// The classic DES known-answer test (key 133457799BBCDFF1, plaintext 0123456789ABCDEF,
    /// ciphertext 85E813540F0AB405). VNC reverses the bits of the key bytes, so the test
    /// passes the reversed bytes as the "password".
    #[test]
    fn matches_the_des_known_answer() {
        let key: Vec<u8> = hex("133457799BBCDFF1")
            .into_iter()
            .map(u8::reverse_bits)
            .collect();
        let mut challenge = [0u8; 16];
        challenge[..8].copy_from_slice(&hex("0123456789ABCDEF"));
        challenge[8..].copy_from_slice(&hex("0123456789ABCDEF"));
        let out = response(&challenge, &key);
        assert_eq!(&out[..8], hex("85E813540F0AB405").as_slice());
        assert_eq!(&out[8..], hex("85E813540F0AB405").as_slice());
    }

    #[test]
    fn only_the_first_eight_bytes_of_the_password_count() {
        let challenge = [7u8; 16];
        assert_eq!(
            response(&challenge, b"12345678"),
            response(&challenge, b"12345678-and-more")
        );
        assert_ne!(
            response(&challenge, b"12345678"),
            response(&challenge, b"1234567x")
        );
    }
}
