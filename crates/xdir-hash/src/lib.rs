//! Sumas de verificación (SHA-256 y MD5) de lo que se lee por bloques, con avance y corte.

use std::io::{self, Read};
use std::ops::ControlFlow;

#[cfg(test)]
mod tests;

/// Lo que se lee de una vez (1 MB).
pub const BLOCK: usize = 1 << 20;

/// Los algoritmos que se ofrecen.
#[derive(Clone, Copy, PartialEq, Eq, Debug)]
pub enum Algo {
    Sha256,
    Md5,
}

/// El estado de una suma en curso.
enum State {
    Sha256(sha2::Sha256),
    Md5(md5::Md5),
}

impl State {
    fn new(algo: Algo) -> State {
        use sha2::Digest;
        match algo {
            Algo::Sha256 => State::Sha256(sha2::Sha256::new()),
            Algo::Md5 => State::Md5(md5::Md5::new()),
        }
    }

    fn update(&mut self, data: &[u8]) {
        use sha2::Digest;
        match self {
            State::Sha256(h) => h.update(data),
            State::Md5(h) => h.update(data),
        }
    }

    fn finish(self) -> String {
        use sha2::Digest;
        match self {
            State::Sha256(h) => hex(&h.finalize()),
            State::Md5(h) => hex(&h.finalize()),
        }
    }
}

/// Llena `buf` todo lo que se pueda (menos solo al final): una lectura corta de la red o una
/// interrupción no cortan el bloque. Devuelve cuánto leyó (0: se terminó).
pub fn read_block(r: &mut dyn Read, buf: &mut [u8]) -> io::Result<usize> {
    let mut n = 0;
    while n < buf.len() {
        match r.read(&mut buf[n..]) {
            Ok(0) => break,
            Ok(k) => n += k,
            Err(e) if e.kind() == io::ErrorKind::Interrupted => {}
            Err(e) => return Err(e),
        }
    }
    Ok(n)
}

/// La suma de lo que da `r`, leído de a bloques de `BLOCK`, en hexadecimal en minúscula.
/// `progress` recibe lo leído en total después de cada bloque; `Break` corta y devuelve `None`.
pub fn hash_reader(r: &mut dyn Read, algo: Algo, progress: &mut dyn FnMut(u64) -> ControlFlow<()>) -> io::Result<Option<String>> {
    let mut state = State::new(algo);
    let mut buf = vec![0u8; BLOCK];
    let mut total = 0u64;
    loop {
        let n = read_block(r, &mut buf)?;
        if n == 0 {
            break;
        }
        state.update(&buf[..n]);
        total += n as u64;
        if progress(total).is_break() {
            return Ok(None);
        }
        if n < BLOCK {
            break;
        }
    }
    Ok(Some(state.finish()))
}

/// Bytes en hexadecimal en minúscula.
pub fn hex(bytes: &[u8]) -> String {
    const DIGITS: &[u8; 16] = b"0123456789abcdef";
    let mut s = String::with_capacity(bytes.len() * 2);
    for b in bytes {
        s.push(DIGITS[(b >> 4) as usize] as char);
        s.push(DIGITS[(b & 0xF) as usize] as char);
    }
    s
}
