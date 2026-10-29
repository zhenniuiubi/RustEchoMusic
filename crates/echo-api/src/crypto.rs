//! 酷狗 API 签名与加解密（对齐 KuGouMusicApi `util/helper.js` + `util/crypto.js`）。

use aes::cipher::{block_padding::Pkcs7, BlockDecryptMut, BlockEncryptMut, KeyIvInit};
use base64::Engine;
use md5::{Digest, Md5};
use rand::Rng;

type Aes128CbcEnc = cbc::Encryptor<aes::Aes128>;
type Aes128CbcDec = cbc::Decryptor<aes::Aes128>;
type Aes256CbcEnc = cbc::Encryptor<aes::Aes256>;
type Aes256CbcDec = cbc::Decryptor<aes::Aes256>;

// ===== 盐值（来自 KuGouMusicApi util/helper.js） =====
/// Android 标准版签名盐
pub const ANDROID_SALT: &str = "OIlwieks28dk2k092lksi2UIkp";
/// Android 概念版(lite)签名盐
pub const ANDROID_SALT_LITE: &str = "LnT6xpN3khm36zse0QzvmgTZ3waWdRSA";
/// 设备注册签名盐
pub const REGISTER_SALT: &str = "1014";
/// signKey 盐（标准版）
pub const KEY_SALT: &str = "57ae12eb6890223e355ccfcb74edf70d";
/// signKey 盐（概念版）/// 通用 sign 盐/// 云盘 key 盐
pub const CLOUD_KEY_SALT: &str = "ebd1ac3134c880bda6a2194537843caa0162e2e7";
/// Web 签名盐
pub const WEB_SALT: &str = "NVPh5oo715z5DIWAeQlhMDsWXXQV4hwt";

/// KuGouMusicApi `config.json`：标准版 appid/clientver
pub const APPID: u32 = 1005;
pub const CLIENTVER: u32 = 20489;
/// 概念版 appid/clientver
pub const LITE_APPID: u32 = 3116;
pub const LITE_CLIENTVER: u32 = 11440;

/// 酷狗 RSA 公钥（标准版，PEM）
pub const RSA_PUBLIC_KEY: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDIAG7QOELSYoIJvTFJhMpe1s/gbjDJX51HBNnEl5HXqTW6lQ7LC8jr9fWZTwusknp+sVGzwd40MwP6U5yDE27M/X1+UR4tvOGOqp94TJtQ1EPnWGWXngpeIW5GxoQGao1rmYWAu6oi1z9XkChrsUdC6DJE5E221wf/4WLFxwAtRQIDAQAB";
/// 酷狗 RSA 公钥（概念版）
pub const RSA_PUBLIC_KEY_LITE: &str = "MIGfMA0GCSqGSIb3DQEBAQUAA4GNADCBiQKBgQDECi0Np2UR87scwrvTr72L6oO01rBbbBPriSDFPxr3Z5syug0O24QyQO8bg27+0+4kBzTBTBOZ/WWU0WryL1JSXRTXLgFVxtzIY41Pe7lPOgsfTCn5kZcvKhYKJesKnnJDNr5/abvTGf+rHG3YRwsCHcQ08/q6ifSioBszvb3QiwIDAQAB";

pub fn md5_hex(input: &str) -> String {
    let mut hasher = Md5::new();
    hasher.update(input.as_bytes());
    hex::encode(hasher.finalize())
}

/// Android 版 signature：`md5(盐 + Σ("k=v") + body + 盐)`，参数按 key 排序、无分隔符。
pub fn signature_android(sorted_params: &[(String, String)], body: &str, lite: bool) -> String {
    let salt = if lite { ANDROID_SALT_LITE } else { ANDROID_SALT };
    let mut s = String::new();
    for (k, v) in sorted_params {
        s.push_str(k);
        s.push('=');
        s.push_str(v);
    }
    md5_hex(&format!("{salt}{s}{body}{salt}"))
}

/// Web 版 signature：`md5(盐 + Σ排序后的 "k=v" + body + 盐)`。
/// 注意 JS 实现是对 `k=v` 字符串整体排序（与按 key 排序不完全等价）。
pub fn signature_web(sorted_params: &[(String, String)], body: &str) -> String {
    let mut parts: Vec<String> = sorted_params
        .iter()
        .map(|(k, v)| format!("{k}={v}"))
        .collect();
    parts.sort();
    md5_hex(&format!("{WEB_SALT}{}{body}{WEB_SALT}", parts.join("")))
}

/// 设备注册 signature：取参数值排序拼接，`md5("1014" + 值串 + "1014")`。
pub fn signature_register(values: &mut Vec<String>) -> String {
    values.sort();
    md5_hex(&format!(
        "{REGISTER_SALT}{}{REGISTER_SALT}",
        values.join("")
    ))
}

/// 请求密钥 key：`md5(hash + 盐 + appid + mid + userid)`。
/// 请求密钥（signKey）。
///
/// 歌曲地址接口的 key 用标准版盐值+ appid 计算；概念版（lite）盐值会返回
/// `error_code 31833`。应用统一走标准版客户端（`lite = false`），这里保留
/// 参数仅为对齐调用方签名。
pub fn sign_key(hash: &str, mid: &str, userid: u64, appid: u32, _lite: bool) -> String {
    md5_hex(&format!("{hash}{KEY_SALT}{appid}{mid}{userid}"))
}

/// 参数密钥 key：`md5(appid + 盐 + clientver + data)`。
pub fn sign_params_key(data: &str, appid: u32, clientver: u32, lite: bool) -> String {
    let salt = if lite { ANDROID_SALT_LITE } else { ANDROID_SALT };
    md5_hex(&format!("{appid}{salt}{clientver}{data}"))
}

/// 云盘 key：`md5("musicclound" + hash + pid + 盐)`。
pub fn sign_cloud_key(hash: &str, pid: &str) -> String {
    md5_hex(&format!("musicclound{hash}{pid}{CLOUD_KEY_SALT}"))
}

/// 24 位随机小写串（song_url 的 dfid 覆盖用）。
pub fn random_string_24() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..24)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// 6 位随机小写串（playlist AES key 种子）。
fn random_string_6() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz";
    let mut rng = rand::rng();
    (0..6)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// AES-128-CBC(PKCS7) 加密，返回 base64 密文。
pub fn aes_cbc_encrypt_b64(key: &[u8], iv: &[u8], plaintext: &[u8]) -> String {
    let enc = Aes128CbcEnc::new(key.into(), iv.into());
    let ct = enc.encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    base64::engine::general_purpose::STANDARD.encode(ct)
}

/// AES-128-CBC(PKCS7) 加密，返回 32 位小写 hex（登录 t1/t2 用）。
///
/// 注意：key 必须正好 16 字节。传入 32 字节 key 会在这里直接 panic（而不是在
/// generic-array 里抛 `assertion left == right failed` 这种看不出原因的断言），
/// 32 字节 key 的场景请用 [`aes256_cbc_encrypt_hex`]。
#[allow(dead_code)]
pub fn aes_cbc_encrypt_hex(key: &[u8], iv: &[u8], plaintext: &[u8]) -> String {
    assert_eq!(
        key.len(),
        16,
        "aes_cbc_encrypt_hex 是 AES-128，key 必须 16 字节（当前 {}）；32 字节 key 请用 aes256_cbc_encrypt_hex",
        key.len()
    );
    assert_eq!(
        iv.len(),
        16,
        "aes_cbc_encrypt_hex 的 iv 必须 16 字节（当前 {}）",
        iv.len()
    );
    let enc = Aes128CbcEnc::new(key.into(), iv.into());
    let ct = enc.encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    hex::encode(ct)
}

/// AES-256-CBC(PKCS7) 加密，返回 hex（固定 key/iv 的登录态加密用，如 `login_by_token` 的 p3）。
pub fn aes256_cbc_encrypt_hex(key: &[u8], iv: &[u8], plaintext: &[u8]) -> String {
    let enc = Aes256CbcEnc::new(key.into(), iv.into());
    let ct = enc.encrypt_padded_vec_mut::<Pkcs7>(plaintext);
    hex::encode(ct)
}

/// AES-128-CBC(PKCS7) 解密 base64 密文。
pub fn aes_cbc_decrypt_b64(key: &[u8], iv: &[u8], b64: &str) -> anyhow::Result<Vec<u8>> {
    let ct = base64::engine::general_purpose::STANDARD.decode(b64)?;
    let dec = Aes128CbcDec::new(key.into(), iv.into());
    let pt = dec
        .decrypt_padded_vec_mut::<Pkcs7>(&ct)
        .map_err(|e| anyhow::anyhow!("aes unpad: {e}"))?;
    Ok(pt)
}

/// playlistAesEncrypt：随机 6 位种子，`key = md5(seed)[0:16]`，`iv = md5(seed)[16:32]`。
/// 返回 `(seed, base64密文)`。
pub fn playlist_aes_encrypt(plaintext: &str) -> (String, String) {
    let seed = random_string_6();
    let digest = md5_hex(&seed);
    let (key, iv) = digest.split_at(16);
    let ct = aes_cbc_encrypt_b64(key.as_bytes(), iv.as_bytes(), plaintext.as_bytes());
    (seed, ct)
}

/// playlistAesDecrypt：按 seed 派生 key/iv 解密 base64 密文。
pub fn playlist_aes_decrypt(seed: &str, b64: &str) -> anyhow::Result<Vec<u8>> {
    let digest = md5_hex(seed);
    let (key, iv) = digest.split_at(16);
    aes_cbc_decrypt_b64(key.as_bytes(), iv.as_bytes(), b64)
}

/// 16 位随机小写串（登录 AES 种子，对齐 `randomString(16).toLowerCase()`）。
fn random_string_16() -> String {
    const ALPHABET: &[u8] = b"abcdefghijklmnopqrstuvwxyz0123456789";
    let mut rng = rand::rng();
    (0..16)
        .map(|_| ALPHABET[rng.random_range(0..ALPHABET.len())] as char)
        .collect()
}

/// 登录用 AES-256-CBC(hex)：`key = md5(tempKey)`(32 字节)，`iv = md5(tempKey)[16..32]`。
/// 返回 `(hex 密文, tempKey)`；tempKey 同时用于 RSA 加密与响应解密。
pub fn login_aes_encrypt(plaintext: &str) -> (String, String) {
    let temp_key = random_string_16();
    let digest = md5_hex(&temp_key);
    let key = &digest[..32];
    let iv = &digest[16..32];
    let enc = Aes256CbcEnc::new(key.as_bytes().into(), iv.as_bytes().into());
    let ct = enc.encrypt_padded_vec_mut::<Pkcs7>(plaintext.as_bytes());
    (hex::encode(ct), temp_key)
}

/// 登录响应解密：`secu_params`(hex) + tempKey → 明文 JSON 串。
pub fn login_aes_decrypt(hex_str: &str, temp_key: &str) -> anyhow::Result<String> {
    let digest = md5_hex(temp_key);
    let key = &digest[..32];
    let iv = &digest[16..32];
    let ct = hex::decode(hex_str.trim())?;
    let dec = Aes256CbcDec::new(key.as_bytes().into(), iv.as_bytes().into());
    let pt = dec
        .decrypt_padded_vec_mut::<Pkcs7>(&ct)
        .map_err(|e| anyhow::anyhow!("login aes unpad: {e}"))?;
    Ok(String::from_utf8_lossy(&pt).to_string())
}

/// RSA PKCS#1 v1.5 加密（DER SubjectPublicKeyInfo），返回大写 hex。
/// 对应 KuGouMusicApi 的 `rsaEncrypt2`。
pub fn rsa_encrypt_hex(plaintext: &[u8], lite: bool) -> anyhow::Result<String> {
    use rsa::pkcs8::DecodePublicKey;
    let pem = if lite { RSA_PUBLIC_KEY_LITE } else { RSA_PUBLIC_KEY };
    let der = base64::engine::general_purpose::STANDARD.decode(pem)?;
    let key = rsa::RsaPublicKey::from_public_key_der(&der)?;
    let mut rng = rsa::rand_core::OsRng;
    let encrypted = key.encrypt(&mut rng, rsa::pkcs1v15::Pkcs1v15Encrypt, plaintext)?;
    Ok(hex::encode_upper(encrypted))
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn md5_known() {
        assert_eq!(md5_hex("abc"), "900150983cd24fb0d6963f7d28e17f72");
    }
}
