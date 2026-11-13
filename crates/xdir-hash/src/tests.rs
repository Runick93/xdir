use std::io::{self, Read};
use std::ops::ControlFlow;

use super::*;

fn hash_of(data: &[u8], algo: Algo) -> String {
    hash_reader(&mut &data[..], algo, &mut |_| ControlFlow::Continue(())).unwrap().unwrap()
}

/// Vectores conocidos de SHA-256 (FIPS 180-2 y el vacío).
#[test]
fn sha256_known_answers() {
    assert_eq!(hash_of(b"", Algo::Sha256), "e3b0c44298fc1c149afbf4c8996fb92427ae41e4649b934ca495991b7852b855");
    assert_eq!(hash_of(b"abc", Algo::Sha256), "ba7816bf8f01cfea414140de5dae2223b00361a396177a9cb410ff61f20015ad");
    assert_eq!(
        hash_of(b"abcdbcdecdefdefgefghfghighijhijkijkljklmklmnlmnomnopnopq", Algo::Sha256),
        "248d6a61d20638b8e5c026930c3e6039a33ce45964ff2167f6ecedd419db06c1"
    );
    // Un millón de «a»: más de un bloque.
    let million = vec![b'a'; 1_000_000];
    assert_eq!(hash_of(&million, Algo::Sha256), "cdc76e5c9914fb9281a1c7e284d73e67f1809a48a497200e046d39ccc7112cd0");
}

/// Vectores conocidos de MD5 (RFC 1321).
#[test]
fn md5_known_answers() {
    assert_eq!(hash_of(b"", Algo::Md5), "d41d8cd98f00b204e9800998ecf8427e");
    assert_eq!(hash_of(b"abc", Algo::Md5), "900150983cd24fb0d6963f7d28e17f72");
    assert_eq!(hash_of(b"message digest", Algo::Md5), "f96b697d7cb7938d525a2f31aaf161d0");
    assert_eq!(
        hash_of(b"12345678901234567890123456789012345678901234567890123456789012345678901234567890", Algo::Md5),
        "57edf4a22be3c955ac49da2e2107b67a"
    );
    let million = vec![b'a'; 1_000_000];
    assert_eq!(hash_of(&million, Algo::Md5), "7707d6ae4e027c70eea2a935c2296f21");
}

/// Un lector que entrega de a pedazos irregulares (como la red) y a veces se interrumpe.
struct Chunky<'a> {
    data: &'a [u8],
    step: usize,
    calls: usize,
}

impl Read for Chunky<'_> {
    fn read(&mut self, buf: &mut [u8]) -> io::Result<usize> {
        self.calls += 1;
        if self.calls.is_multiple_of(5) {
            return Err(io::Error::new(io::ErrorKind::Interrupted, "señal"));
        }
        let n = self.step.min(buf.len()).min(self.data.len());
        buf[..n].copy_from_slice(&self.data[..n]);
        self.data = &self.data[n..];
        self.step = self.step % 13 + 3;
        Ok(n)
    }
}

/// Leer en pedazos chicos e interrumpidos da la misma suma.
#[test]
fn streaming_in_pieces_gives_the_same_sum() {
    let data: Vec<u8> = (0..300_000u32).map(|i| (i * 7 % 251) as u8).collect();
    for algo in [Algo::Sha256, Algo::Md5] {
        let mut r = Chunky { data: &data, step: 1, calls: 0 };
        let got = hash_reader(&mut r, algo, &mut |_| ControlFlow::Continue(())).unwrap().unwrap();
        assert_eq!(got, hash_of(&data, algo));
    }
}

/// El avance llega por bloque de 1 MB, con lo leído en total, y la última vez con todo.
#[test]
fn progress_per_block() {
    let data = vec![1u8; 2 * BLOCK + 10];
    let mut seen = Vec::new();
    hash_reader(&mut &data[..], Algo::Sha256, &mut |b| {
        seen.push(b);
        ControlFlow::Continue(())
    })
    .unwrap()
    .unwrap();
    assert_eq!(seen, [BLOCK as u64, 2 * BLOCK as u64, 2 * BLOCK as u64 + 10]);
}

/// Cortar desde el avance deja de leer y no da suma.
#[test]
fn cancel_stops_reading() {
    let data = vec![2u8; 3 * BLOCK];
    let mut r = &data[..];
    let got = hash_reader(&mut r, Algo::Md5, &mut |_| ControlFlow::Break(())).unwrap();
    assert_eq!(got, None);
    assert_eq!(r.len(), 2 * BLOCK, "leyó un solo bloque");
}

/// Un error de lectura vuelve como error.
#[test]
fn read_error_is_returned() {
    struct Broken;
    impl Read for Broken {
        fn read(&mut self, _: &mut [u8]) -> io::Result<usize> {
            Err(io::Error::other("disco"))
        }
    }
    assert!(hash_reader(&mut Broken, Algo::Sha256, &mut |_| ControlFlow::Continue(())).is_err());
}

#[test]
fn hex_is_lowercase() {
    assert_eq!(hex(&[0x00, 0xAB, 0x0F, 0xF0]), "00ab0ff0");
}
