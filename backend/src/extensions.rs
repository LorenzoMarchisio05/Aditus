//! Chrome extensions: unpacked folders under `<data>/extensions/<id>`, loaded at startup with
//! `--load-extension` (CEF has no extension API; Chromium's own extension system does the work).
//! Content scripts, service workers and declarativeNetRequest run; `chrome.tabs.query` sees no tabs.
use base64::{engine::general_purpose::STANDARD, Engine};
use serde_json::{json, Value};
use sha2::{Digest, Sha256};
use std::{
    fs,
    io::{Read, Write},
    path::{Path, PathBuf},
};

const MAX_CRX: u64 = 120 << 20;

pub fn root() -> PathBuf {
    crate::data_dir().join("extensions")
}

/// The 32-letter id inside a Web Store URL, or a bare id.
pub fn parse_id(input: &str) -> Option<String> {
    input.split(|c: char| !c.is_ascii_alphanumeric()).find(|t| t.len() == 32 && t.bytes().all(|b| (b'a'..=b'p').contains(&b))).map(str::to_string)
}

/// `have`: the installed version; the store then answers with nothing when it is current.
fn fetch(id: &str, have: Option<&str>) -> Result<Vec<u8>, String> {
    let v = have.map(|v| format!("%26v%3D{}", v.replace(|c: char| !(c.is_ascii_digit() || c == '.'), ""))).unwrap_or_default();
    let url = format!("https://clients2.google.com/service/update2/crx?response=redirect&prodversion=156.0.0.0&acceptformat=crx2,crx3&x=id%3D{id}{v}%26uc");
    let mut buf = Vec::new();
    ureq::get(&url).call().map_err(|e| e.to_string())?.into_reader().take(MAX_CRX).read_to_end(&mut buf).map_err(|e| e.to_string())?;
    Ok(buf)
}

fn varint(b: &[u8], i: &mut usize) -> Option<u64> {
    let (mut v, mut shift) = (0u64, 0);
    loop {
        let byte = *b.get(*i)?;
        *i += 1;
        v |= ((byte & 0x7f) as u64) << shift;
        if byte & 0x80 == 0 {
            return Some(v);
        }
        shift += 7;
        if shift > 63 {
            return None;
        }
    }
}

/// (field number, bytes) of every length-delimited field; None on any other wire type.
fn fields(b: &[u8]) -> Option<Vec<(u64, &[u8])>> {
    let (mut i, mut out) = (0, Vec::new());
    while i < b.len() {
        let tag = varint(b, &mut i)?;
        if tag & 7 != 2 {
            return None;
        }
        let len = varint(b, &mut i)? as usize;
        out.push((tag >> 3, b.get(i..i.checked_add(len)?)?));
        i += len;
    }
    Some(out)
}

/// One DER element at the start of `b`: (tag, content, rest).
fn der(b: &[u8]) -> Option<(u8, &[u8], &[u8])> {
    let (&tag, b) = b.split_first()?;
    let (&l, b) = b.split_first()?;
    let (len, b) = if l < 0x80 {
        (l as usize, b)
    } else {
        let n = (l & 0x7f) as usize;
        let bytes = b.get(..n).filter(|_| n <= 4)?;
        (bytes.iter().fold(0usize, |a, &x| a << 8 | x as usize), &b[n..])
    };
    Some((tag, b.get(..len)?, &b[len..]))
}

/// The raw key inside a SubjectPublicKeyInfo (PKCS#1 RSAPublicKey, or the EC point), which is what ring takes.
fn spki_bits(spki: &[u8]) -> Option<&[u8]> {
    let (0x30, body, _) = der(spki)? else { return None };
    let (0x30, _, rest) = der(body)? else { return None };
    let (0x03, bits, _) = der(rest)? else { return None };
    bits.strip_prefix(&[0])
}

/// CRX3 signature check: the proof signs "CRX3 SignedData\0" + le32(len) + signed_header_data + zip.
fn verify(alg: &'static dyn ring::signature::VerificationAlgorithm, spki: &[u8], sig: &[u8], signed: &[u8], zip: &[u8]) -> bool {
    let Some(key) = spki_bits(spki) else { return false };
    let mut msg = b"CRX3 SignedData\0".to_vec();
    msg.extend((signed.len() as u32).to_le_bytes());
    msg.extend(signed);
    msg.extend(zip);
    ring::signature::UnparsedPublicKey::new(alg, key).verify(&msg, sig).is_ok()
}

/// Splits a CRX3 file into its zip and the public key (base64) that its id was derived from.
fn split_crx3(crx: &[u8], want_id: &str) -> Result<(Vec<u8>, String), String> {
    let bad = || "not a CRX3 extension file".to_string();
    if crx.get(..4) != Some(b"Cr24") || crx.get(4..8) != Some(&3u32.to_le_bytes()[..]) {
        return Err(bad());
    }
    let hl = u32::from_le_bytes(crx.get(8..12).and_then(|b| b.try_into().ok()).ok_or_else(bad)?) as usize;
    let header = crx.get(12..12usize.checked_add(hl).ok_or_else(bad)?).ok_or_else(bad)?;
    let zip = crx[12 + hl..].to_vec();
    let top = fields(header).ok_or_else(bad)?;
    let crx_id = top.iter().find(|f| f.0 == 10000).and_then(|f| fields(f.1)).and_then(|s| s.iter().find(|f| f.0 == 1).map(|f| f.1.to_vec())).ok_or_else(bad)?;
    let id: String = crx_id.iter().flat_map(|b| [b >> 4, b & 15]).map(|n| (b'a' + n) as char).collect();
    if id != want_id {
        return Err("the file is for a different extension".into());
    }
    let signed = top.iter().find(|f| f.0 == 10000).map(|f| f.1).ok_or_else(bad)?;
    // The proof whose key hashes to the id carries the key the manifest needs to keep that id, and must sign the zip.
    for (n, proof) in &top {
        if *n != 2 && *n != 3 {
            continue;
        }
        let p = fields(proof).unwrap_or_default();
        let get = |k| p.iter().find(|f| f.0 == k).map(|f| f.1);
        let (Some(key), Some(sig)) = (get(1), get(2)) else { continue };
        if Sha256::digest(key)[..16] != crx_id[..] {
            continue;
        }
        let alg: &'static dyn ring::signature::VerificationAlgorithm = if *n == 2 { &ring::signature::RSA_PKCS1_2048_8192_SHA256 } else { &ring::signature::ECDSA_P256_SHA256_ASN1 };
        if !verify(alg, key, sig, signed, &zip) {
            return Err("the extension's signature is invalid".into());
        }
        return Ok((zip, STANDARD.encode(key)));
    }
    Err("the file has no key matching its id".into())
}

fn unzip(zip: &[u8], to: &Path) -> Result<(), String> {
    let mut a = zip::ZipArchive::new(std::io::Cursor::new(zip)).map_err(|e| e.to_string())?;
    if a.len() > 20_000 {
        return Err("extension has too many files".into());
    }
    let mut total = 0u64;
    for i in 0..a.len() {
        let mut f = a.by_index(i).map_err(|e| e.to_string())?;
        let Some(rel) = f.enclosed_name() else { continue };
        let dest = to.join(rel);
        if f.is_dir() {
            fs::create_dir_all(&dest).map_err(|e| e.to_string())?;
            continue;
        }
        if let Some(d) = dest.parent() {
            fs::create_dir_all(d).map_err(|e| e.to_string())?;
        }
        let mut out = fs::File::create(&dest).map_err(|e| e.to_string())?;
        // Count what is actually written: the declared size in the zip header can lie.
        let budget = (400u64 << 20) - total;
        total += std::io::copy(&mut f.by_ref().take(budget + 1), &mut out).map_err(|e| e.to_string())?;
        if total > 400 << 20 {
            return Err("extension is too large".into());
        }
        out.flush().ok();
    }
    Ok(())
}

fn staging(id: &str) -> PathBuf {
    root().join(".staging").join(id)
}

fn text(m: &Value, k: &str) -> String {
    m[k].as_str().unwrap_or("").to_string()
}

/// `__MSG_key__` from the extension's own locale files.
fn localized(dir: &Path, m: &Value, v: &str) -> String {
    let Some(key) = v.strip_prefix("__MSG_").and_then(|k| k.strip_suffix("__")) else { return v.to_string() };
    let locale = m["default_locale"].as_str().unwrap_or("en");
    let msgs: Value = fs::read_to_string(dir.join("_locales").join(locale).join("messages.json")).ok().and_then(|s| serde_json::from_str(&s).ok()).unwrap_or(Value::Null);
    msgs.as_object().and_then(|o| o.iter().find(|(k, _)| k.eq_ignore_ascii_case(key))).and_then(|(_, v)| v["message"].as_str()).unwrap_or(v).to_string()
}

fn describe(dir: &Path, id: &str) -> Option<Value> {
    let m: Value = serde_json::from_str(&fs::read_to_string(dir.join("manifest.json")).ok()?).ok()?;
    let list = |k: &str| m[k].as_array().map(|a| a.iter().filter_map(|v| v.as_str()).collect::<Vec<_>>()).unwrap_or_default();
    let perms = list("permissions");
    let action = if m["action"].is_object() { &m["action"] } else { &m["browser_action"] };
    let options = m["options_page"].as_str().or_else(|| m["options_ui"]["page"].as_str());
    Some(json!({
        "id": id,
        "name": localized(dir, &m, &text(&m, "name")),
        "version": text(&m, "version"),
        "permissions": perms,
        "hosts": list("host_permissions"),
        "popup": action["default_popup"],
        "options": options,
        "mv": m["manifest_version"],
    }))
}

/// Downloads `input` (a Web Store URL or id) into a staging folder and says what it asks for.
pub fn stage(input: &str) -> Result<Value, String> {
    let id = parse_id(input).ok_or("That is not a Chrome Web Store link or extension id.")?;
    stage_id(&id, None)
}

fn stage_id(id: &str, have: Option<&str>) -> Result<Value, String> {
    let (zip, key) = split_crx3(&fetch(id, have)?, id)?;
    let dir = staging(id);
    let _ = fs::remove_dir_all(&dir);
    fs::create_dir_all(&dir).map_err(|e| e.to_string())?;
    let done = (|| {
        unzip(&zip, &dir)?;
        let path = dir.join("manifest.json");
        let mut m: Value = serde_json::from_str(&fs::read_to_string(&path).map_err(|e| e.to_string())?).map_err(|e| e.to_string())?;
        if m["manifest_version"].as_u64() != Some(3) {
            return Err("Only Manifest V3 extensions work in this version of Chromium.".to_string());
        }
        m["key"] = json!(key);
        fs::write(&path, m.to_string()).map_err(|e| e.to_string())
    })();
    if let Err(e) = done {
        let _ = fs::remove_dir_all(&dir);
        return Err(e);
    }
    describe(&dir, id).ok_or_else(|| "The extension has no readable manifest.".to_string())
}

fn valid_id(id: &str) -> bool {
    id.len() == 32 && id.bytes().all(|b| (b'a'..=b'p').contains(&b))
}

pub fn commit(id: &str) -> Result<(), String> {
    if !valid_id(id) {
        return Err("bad id".into());
    }
    let to = root().join(id);
    let _ = fs::remove_dir_all(&to);
    fs::rename(staging(id), &to).map_err(|e| e.to_string())
}

pub fn discard(id: &str) {
    if valid_id(id) {
        let _ = fs::remove_dir_all(staging(id));
    }
}

/// Installed extensions that have a newer store version with no new permissions are replaced on disk;
/// Chromium reads them at start-up, so the new code runs after the next restart.
fn update_all() {
    let perms = |d: &Value| -> Vec<String> { ["permissions", "hosts"].iter().flat_map(|k| d[k].as_array().cloned().unwrap_or_default()).filter_map(|v| v.as_str().map(String::from)).collect() };
    for id in ids() {
        let Some(old) = describe(&root().join(&id), &id) else { continue };
        let have = text(&old, "version");
        // An up-to-date extension makes the store answer with nothing, which fails to parse: no update.
        let Ok(new) = stage_id(&id, Some(&have)) else {
            discard(&id);
            continue;
        };
        if text(&new, "version") != have && perms(&new).iter().all(|p| perms(&old).contains(p)) {
            let _ = crate::applog::check("extension update commit", commit(&id));
        } else {
            discard(&id);
        }
    }
}

/// A minute after start-up, then every 6 hours; never blocks anything.
pub fn init_updates() {
    std::thread::spawn(|| loop {
        std::thread::sleep(std::time::Duration::from_secs(60));
        update_all();
        std::thread::sleep(std::time::Duration::from_secs(6 * 3600));
    });
}

fn disabled() -> Vec<String> {
    fs::read_to_string(root().join("state.json"))
        .ok()
        .and_then(|s| serde_json::from_str::<Value>(&s).ok())
        .and_then(|v| v["disabled"].as_array().map(|a| a.iter().filter_map(|x| x.as_str().map(String::from)).collect()))
        .unwrap_or_default()
}

pub fn set_enabled(id: &str, on: bool) {
    let mut d = disabled();
    d.retain(|x| x != id);
    if !on && valid_id(id) {
        d.push(id.to_string());
    }
    let _ = fs::create_dir_all(root());
    let _ = crate::applog::check("extensions state", crate::config::write_atomic(&root().join("state.json"), json!({ "disabled": d }).to_string()));
}

pub fn remove(id: &str) {
    if valid_id(id) {
        let _ = fs::remove_dir_all(root().join(id));
        set_enabled(id, true);
    }
}

fn ids() -> Vec<String> {
    let mut v: Vec<String> = fs::read_dir(root()).into_iter().flatten().flatten().filter_map(|e| e.file_name().into_string().ok()).filter(|n| valid_id(n)).collect();
    v.sort();
    v
}

pub fn list() -> Vec<Value> {
    let off = disabled();
    ids()
        .iter()
        .filter_map(|id| {
            describe(&root().join(id), id).map(|mut d| {
                d["enabled"] = json!(!off.contains(id));
                d
            })
        })
        .collect()
}

/// For `--load-extension`: the enabled extensions' folders, comma separated.
pub fn load_arg() -> Option<String> {
    let off = disabled();
    let dirs: Vec<String> = ids().into_iter().filter(|id| !off.contains(id)).map(|id| root().join(id).to_string_lossy().into_owned()).collect();
    (!dirs.is_empty()).then(|| dirs.join(","))
}

/// `chrome-extension://id/path` for the popup or options page.
pub fn page_url(id: &str, which: &str) -> Option<String> {
    let d = list().into_iter().find(|d| d["id"] == id)?;
    let p = d[which].as_str()?.trim_start_matches('/');
    Some(format!("chrome-extension://{id}/{p}"))
}

#[cfg(test)]
mod tests {
    use super::*;

    fn varint_bytes(mut n: usize) -> Vec<u8> {
        let mut v = vec![];
        while n >= 0x80 {
            v.push((n & 0x7f) as u8 | 0x80);
            n >>= 7;
        }
        v.push(n as u8);
        v
    }
    fn field(n: u64, b: &[u8]) -> Vec<u8> {
        let mut v = varint_bytes(((n << 3) | 2) as usize);
        v.extend(varint_bytes(b.len()));
        v.extend(b);
        v
    }

    #[test]
    fn ids() {
        let id = "nngceckbapebfimnlniiiahkandclblb";
        assert_eq!(parse_id(&format!("https://chromewebstore.google.com/detail/bitwarden/{id}?hl=en")).as_deref(), Some(id));
        assert_eq!(parse_id(id).as_deref(), Some(id));
        assert_eq!(parse_id("https://example.com/abc"), None);
    }

    /// A CRX3 signed with a fresh P-256 key; `zip` is what gets signed, `tamper` swaps the shipped bytes.
    fn signed_crx(zip: &[u8], tamper: &[u8]) -> (Vec<u8>, String) {
        use ring::signature::{EcdsaKeyPair, KeyPair, ECDSA_P256_SHA256_ASN1_SIGNING};
        let rng = ring::rand::SystemRandom::new();
        let pkcs8 = EcdsaKeyPair::generate_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, &rng).unwrap();
        let kp = EcdsaKeyPair::from_pkcs8(&ECDSA_P256_SHA256_ASN1_SIGNING, pkcs8.as_ref(), &rng).unwrap();
        let mut spki = hex_prefix();
        spki.extend(kp.public_key().as_ref());
        crx_from(3, &spki, zip, tamper, |msg| kp.sign(&rng, msg).unwrap().as_ref().to_vec())
    }

    /// Test-only 2048-bit RSA key (PKCS#8) and its SubjectPublicKeyInfo, both base64; ring can load but not generate RSA keys.
    const RSA_PKCS8: &str = "MIIEvQIBADANBgkqhkiG9w0BAQEFAASCBKcwggSjAgEAAoIBAQC+XoH0KGWGepHaSYWkkdpcoiZ+gUSS2UKq66bBcsSpp94WMTJhXTslT/xAi8P4ADrtNnliFDIbCi2TdxkH4TggNK3gTthvAznAQ0iefTpV10KOl47+lNag3KH1FFvvpnNTuUPwjFoiy1Q3RS1qro6ErLYmwnkzvwTH28wbrWM80IKSym+Z3/5Fg5/nST2L+773aOrl7p/UMX9v6dBorQ4IDDGLukjZx4HAcB1Z1DU0K5tlh0BpU4Wa0u8B+grwHvi8lCcWQ+Ha0uRzzm+TP8Hyct7H6o75fjppP1p8H1Jj2dFns5skVqDbXFtPW94K7dmlNjS95vcLg+S5eVAX8DAzAgMBAAECggEAD3gjjEEaGubr4jaWPwzBF0Tv5ti/R0WTVstGZjLay0SCVgBSbToDu+D3iREyb/0ozKzgCTnF5o+Zq4GCjbLd+jP9qxwNF1xGPZraFFLtwhr3ZBnpu0NB+5liKcWpQNygQ33rCOmGy4wFQGaFLcWYfCyCsryb17oee9UfDWK2DTNqb7vWi5xiS6yRDPGL1PbgpGjcoWvwzafe9wntwZoYI0bqbPPq/WBjGQ7hKkTUfluMq8BKOfKtqdvhTQ19xy++T6j0MJygob+1qpiRPsZHRDCaSCD3YxdR2xev4hI99yqfDxXYQKsDQBBcvlCYdIlYsoe8456H8APXj/jMQtvIFQKBgQDqVQwPpjSnc9AxOuzOnRQAUdi7MIRXTgYSkCw56fIl+/myo+0+EzfYNQaQ6YEUUzP7hZWllutSm8tu86IWcoKd1pcar+HC6QJgqqmrElImDjBfVsWYNwFTCz8LRECTv9jXPlCIkSJwtqVN24BrL0rb0X8Uofd6Efn4EcD4QB5xJwKBgQDP+MwV5souUR5g7LLsUyujqlJwD8KsoGI+d1YaCez8BtR5k3HOjZRTjITdserTnsvIYkPayt1RYAJ4GoPyoaN+YQ99oIiQzkYfo9RgjGRRPVEc1omTLb9dKx3NS4Zjn5O//H5ia2/MroEHy7bIJyQfN80rb8ZD0QzE1m1p9UjYFQKBgFVtc9wwp3l0iy2nYbVhJm2HaQ+0/Eons6cI0Fa8pv6aAcnSYScEMpOroR0rQ1d5jbXgWVHo1xlimqN0Rl8Ql/WaybN88zzvXWKIVRp8r14YRiI7rAV16BGyQ+WDmFdE1XSCzzE6V+fnbhQYNZTdxNimY0HOWj4yV4LMViEqnu55AoGATEzPM0nU62E/b/fnPOTWodkfHZXqqkK9SIWnxS3XEC/BTElX+5vdyzlGlb9sk+8yfHXwxxieUfiTb1TrC91Zikg50rFO6TGEcJ8+j51yQIvqclYTazfheWEUMfl/eJVXM5yDEcCw57suA5otmQkjK4DheQotBzxvHtg8lB81dnECgYEAsh3ZhX7jFTSIb8FEgn0ZWBEmV/xn9mLskr0TlwldMSV2gk5L1iA1CnpUKQ7HLBUyVmW8/+qFrbOEeApBD9gQlCAZqmiVIDP/YkE/N8Iaz78sdION0E5kmT+cWr6Vbwu4kqbxPsOJL5pRt5ERjbWO2pynwxMtIzScfBvttJDCkho=";
    const RSA_SPKI: &str = "MIIBIjANBgkqhkiG9w0BAQEFAAOCAQ8AMIIBCgKCAQEAvl6B9ChlhnqR2kmFpJHaXKImfoFEktlCquumwXLEqafeFjEyYV07JU/8QIvD+AA67TZ5YhQyGwotk3cZB+E4IDSt4E7YbwM5wENInn06VddCjpeO/pTWoNyh9RRb76ZzU7lD8IxaIstUN0Utaq6OhKy2JsJ5M78Ex9vMG61jPNCCkspvmd/+RYOf50k9i/u+92jq5e6f1DF/b+nQaK0OCAwxi7pI2ceBwHAdWdQ1NCubZYdAaVOFmtLvAfoK8B74vJQnFkPh2tLkc85vkz/B8nLex+qO+X46aT9afB9SY9nRZ7ObJFag21xbT1veCu3ZpTY0veb3C4PkuXlQF/AwMwIDAQAB";

    /// Same, but RSA-signed (proof field 2), the kind the Web Store mostly ships.
    fn signed_crx_rsa(zip: &[u8], tamper: &[u8]) -> (Vec<u8>, String) {
        let kp = ring::signature::RsaKeyPair::from_pkcs8(&STANDARD.decode(RSA_PKCS8).unwrap()).unwrap();
        let rng = ring::rand::SystemRandom::new();
        crx_from(2, &STANDARD.decode(RSA_SPKI).unwrap(), zip, tamper, |msg| {
            let mut sig = vec![0; kp.public().modulus_len()];
            kp.sign(&ring::signature::RSA_PKCS1_SHA256, &rng, msg, &mut sig).unwrap();
            sig
        })
    }

    fn crx_from(proof_field: u64, spki: &[u8], zip: &[u8], tamper: &[u8], sign: impl Fn(&[u8]) -> Vec<u8>) -> (Vec<u8>, String) {
        let crx_id = Sha256::digest(spki)[..16].to_vec();
        let id: String = crx_id.iter().flat_map(|b| [b >> 4, b & 15]).map(|n| (b'a' + n) as char).collect();
        let signed = field(1, &crx_id);
        let mut msg = b"CRX3 SignedData\0".to_vec();
        msg.extend((signed.len() as u32).to_le_bytes());
        msg.extend(&signed);
        msg.extend(zip);
        let sig = sign(&msg);
        let mut proof = field(1, spki);
        proof.extend(field(2, &sig));
        let mut header = field(proof_field, &proof);
        header.extend(field(10000, &signed));
        let mut crx = b"Cr24".to_vec();
        crx.extend(3u32.to_le_bytes());
        crx.extend((header.len() as u32).to_le_bytes());
        crx.extend(&header);
        crx.extend(tamper);
        (crx, id)
    }

    /// DER prefix of a P-256 SubjectPublicKeyInfo, up to the uncompressed point.
    fn hex_prefix() -> Vec<u8> {
        vec![0x30, 0x59, 0x30, 0x13, 0x06, 0x07, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x02, 0x01, 0x06, 0x08, 0x2a, 0x86, 0x48, 0xce, 0x3d, 0x03, 0x01, 0x07, 0x03, 0x42, 0x00]
    }

    #[test]
    fn crx3() {
        let (crx, id) = signed_crx(b"PKzipbytes", b"PKzipbytes");
        let (zip, k) = split_crx3(&crx, &id).unwrap();
        assert_eq!(zip, b"PKzipbytes");
        assert_eq!(Sha256::digest(STANDARD.decode(k).unwrap())[..16], parse_crx_id(&id)[..]);
        assert!(split_crx3(&crx, "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa").is_err());
        assert!(split_crx3(b"Cr24\x02\0\0\0", &id).is_err());
        // Zip swapped after signing: rejected.
        let (bad, id) = signed_crx(b"PKzipbytes", b"PKevilbytes");
        assert!(split_crx3(&bad, &id).unwrap_err().contains("signature"));
    }

    #[test]
    fn crx3_rsa() {
        let (crx, id) = signed_crx_rsa(b"PKzipbytes", b"PKzipbytes");
        assert_eq!(split_crx3(&crx, &id).unwrap().0, b"PKzipbytes");
        let (bad, id) = signed_crx_rsa(b"PKzipbytes", b"PKevilbytes");
        assert!(split_crx3(&bad, &id).unwrap_err().contains("signature"));
    }

    fn parse_crx_id(id: &str) -> Vec<u8> {
        id.as_bytes().chunks(2).map(|c| (c[0] - b'a') << 4 | (c[1] - b'a')).collect()
    }
}
