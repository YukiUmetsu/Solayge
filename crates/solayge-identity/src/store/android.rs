//! Android Keystore-backed storage.
//!
//! The device identity seed is wrapped with an AES-256-GCM key that lives only
//! in the **Android Keystore** (hardware-backed where the device provides it).
//! The wrapped blob is stored in the app's private file directory, so the seed
//! is never at rest in the clear and the wrapping key never leaves the keystore.
//!
//! This module is compiled only for `target_os = "android"`. It reaches the
//! platform through JNI; it does not implement any cryptography itself.

use std::path::{Path, PathBuf};

use jni::objects::{JByteArray, JObject, JValue};
use jni::{JNIEnv, JavaVM};

use super::{sanitize, write_private};
use crate::error::{IdentityError, Result};
use crate::store::SecureStore;

const GCM_TAG_BITS: i32 = 128;
const CIPHER_ENCRYPT_MODE: i32 = 1;
const CIPHER_DECRYPT_MODE: i32 = 2;
/// `KeyProperties.PURPOSE_ENCRYPT | KeyProperties.PURPOSE_DECRYPT`.
const KEY_PURPOSE_ENCRYPT_DECRYPT: i32 = 3;
const GCM_IV_LEN: usize = 12;

/// Android Keystore-backed secret storage.
pub struct AndroidKeystoreStore {
    dir: PathBuf,
    service: String,
}

impl AndroidKeystoreStore {
    /// A store rooted at `dir` using a keystore alias derived from `service`.
    pub fn new(dir: &Path, service: &str) -> Self {
        AndroidKeystoreStore {
            dir: dir.join("android-keystore"),
            service: sanitize(service),
        }
    }

    fn alias(&self) -> String {
        format!("{}.identity.v1", self.service)
    }

    fn value_path(&self, key: &str) -> PathBuf {
        self.dir.join(format!("{}.bin", sanitize(key)))
    }
}

impl SecureStore for AndroidKeystoreStore {
    fn get(&self, key: &str) -> Result<Option<Vec<u8>>> {
        match std::fs::read(self.value_path(key)) {
            Ok(blob) => Ok(Some(unwrap(&self.alias(), &blob)?)),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(None),
            Err(e) => Err(IdentityError::Storage(e.to_string())),
        }
    }

    fn set(&self, key: &str, value: &[u8]) -> Result<()> {
        let wrapped = wrap(&self.alias(), value)?;
        std::fs::create_dir_all(&self.dir).map_err(|e| IdentityError::Storage(e.to_string()))?;
        write_private(&self.value_path(key), &wrapped)
    }

    fn delete(&self, key: &str) -> Result<()> {
        match std::fs::remove_file(self.value_path(key)) {
            Ok(()) => Ok(()),
            Err(e) if e.kind() == std::io::ErrorKind::NotFound => Ok(()),
            Err(e) => Err(IdentityError::Storage(e.to_string())),
        }
    }

    fn backend_name(&self) -> &'static str {
        "android-keystore"
    }
}

fn jerr<E: std::fmt::Debug>(e: E) -> IdentityError {
    IdentityError::Storage(format!("android keystore jni: {e:?}"))
}

fn with_env<T>(f: impl FnOnce(&mut JNIEnv<'_>) -> Result<T>) -> Result<T> {
    let context = ndk_context::android_context();
    let vm = unsafe { JavaVM::from_raw(context.vm().cast()) }.map_err(jerr)?;
    let mut guard = vm.attach_current_thread().map_err(jerr)?;
    f(&mut guard)
}

fn wrap(alias: &str, plaintext: &[u8]) -> Result<Vec<u8>> {
    with_env(|env| {
        let key = get_or_create_key(env, alias)?;
        let cipher = new_cipher(env)?;
        env.call_method(
            &cipher,
            "init",
            "(ILjava/security/Key;)V",
            &[JValue::Int(CIPHER_ENCRYPT_MODE), JValue::Object(&key)],
        )
        .map_err(jerr)?;

        let iv: JByteArray = env
            .call_method(&cipher, "getIV", "()[B", &[])
            .map_err(jerr)?
            .l()
            .map_err(jerr)?
            .into();
        let iv = env.convert_byte_array(&iv).map_err(jerr)?;
        if iv.len() != GCM_IV_LEN {
            return Err(IdentityError::Storage("unexpected GCM IV length".into()));
        }

        let input = env.byte_array_from_slice(plaintext).map_err(jerr)?;
        let output: JByteArray = env
            .call_method(
                &cipher,
                "doFinal",
                "([B)[B",
                &[JValue::Object(AsRef::<JObject>::as_ref(&input))],
            )
            .map_err(jerr)?
            .l()
            .map_err(jerr)?
            .into();
        let ciphertext = env.convert_byte_array(&output).map_err(jerr)?;

        let mut blob = iv;
        blob.extend_from_slice(&ciphertext);
        Ok(blob)
    })
}

fn unwrap(alias: &str, blob: &[u8]) -> Result<Vec<u8>> {
    if blob.len() <= GCM_IV_LEN {
        return Err(IdentityError::Storage("wrapped value is too short".into()));
    }
    let (iv, ciphertext) = blob.split_at(GCM_IV_LEN);
    with_env(|env| {
        let key = get_or_create_key(env, alias)?;
        let cipher = new_cipher(env)?;
        let iv_array = env.byte_array_from_slice(iv).map_err(jerr)?;
        let spec = env
            .new_object(
                "javax/crypto/spec/GCMParameterSpec",
                "(I[B)V",
                &[
                    JValue::Int(GCM_TAG_BITS),
                    JValue::Object(AsRef::<JObject>::as_ref(&iv_array)),
                ],
            )
            .map_err(jerr)?;
        env.call_method(
            &cipher,
            "init",
            "(ILjava/security/Key;Ljava/security/spec/AlgorithmParameterSpec;)V",
            &[
                JValue::Int(CIPHER_DECRYPT_MODE),
                JValue::Object(&key),
                JValue::Object(&spec),
            ],
        )
        .map_err(jerr)?;

        let input = env.byte_array_from_slice(ciphertext).map_err(jerr)?;
        let output: JByteArray = env
            .call_method(
                &cipher,
                "doFinal",
                "([B)[B",
                &[JValue::Object(AsRef::<JObject>::as_ref(&input))],
            )
            .map_err(jerr)?
            .l()
            .map_err(jerr)?
            .into();
        env.convert_byte_array(&output).map_err(jerr)
    })
}

fn new_cipher<'local>(env: &mut JNIEnv<'local>) -> Result<JObject<'local>> {
    let transformation = java_string(env, "AES/GCM/NoPadding")?;
    env.call_static_method(
        "javax/crypto/Cipher",
        "getInstance",
        "(Ljava/lang/String;)Ljavax/crypto/Cipher;",
        &[JValue::Object(&transformation)],
    )
    .map_err(jerr)?
    .l()
    .map_err(jerr)
}

fn get_or_create_key<'local>(env: &mut JNIEnv<'local>, alias: &str) -> Result<JObject<'local>> {
    let key_store = android_key_store(env)?;
    let alias_string = java_string(env, alias)?;

    let contains = env
        .call_method(
            &key_store,
            "containsAlias",
            "(Ljava/lang/String;)Z",
            &[JValue::Object(&alias_string)],
        )
        .map_err(jerr)?
        .z()
        .map_err(jerr)?;

    if !contains {
        generate_aes_key(env, &alias_string)?;
    }

    env.call_method(
        &key_store,
        "getKey",
        "(Ljava/lang/String;[C)Ljava/security/Key;",
        &[
            JValue::Object(&alias_string),
            JValue::Object(&JObject::null()),
        ],
    )
    .map_err(jerr)?
    .l()
    .map_err(jerr)
}

fn android_key_store<'local>(env: &mut JNIEnv<'local>) -> Result<JObject<'local>> {
    let provider = java_string(env, "AndroidKeyStore")?;
    let key_store = env
        .call_static_method(
            "java/security/KeyStore",
            "getInstance",
            "(Ljava/lang/String;)Ljava/security/KeyStore;",
            &[JValue::Object(&provider)],
        )
        .map_err(jerr)?
        .l()
        .map_err(jerr)?;
    env.call_method(
        &key_store,
        "load",
        "(Ljava/security/KeyStore$LoadStoreParameter;)V",
        &[JValue::Object(&JObject::null())],
    )
    .map_err(jerr)?;
    Ok(key_store)
}

fn generate_aes_key(env: &mut JNIEnv<'_>, alias: &JObject<'_>) -> Result<()> {
    let aes = java_string(env, "AES")?;
    let provider = java_string(env, "AndroidKeyStore")?;
    let generator = env
        .call_static_method(
            "javax/crypto/KeyGenerator",
            "getInstance",
            "(Ljava/lang/String;Ljava/lang/String;)Ljavax/crypto/KeyGenerator;",
            &[JValue::Object(&aes), JValue::Object(&provider)],
        )
        .map_err(jerr)?
        .l()
        .map_err(jerr)?;

    let builder = env
        .new_object(
            "android/security/keystore/KeyGenParameterSpec$Builder",
            "(Ljava/lang/String;I)V",
            &[
                JValue::Object(AsRef::<JObject>::as_ref(alias)),
                JValue::Int(KEY_PURPOSE_ENCRYPT_DECRYPT),
            ],
        )
        .map_err(jerr)?;

    let gcm = java_string(env, "GCM")?;
    let modes = string_array(env, &gcm)?;
    let builder = env
        .call_method(
            &builder,
            "setBlockModes",
            "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
            &[JValue::Object(AsRef::<JObject>::as_ref(&modes))],
        )
        .map_err(jerr)?
        .l()
        .map_err(jerr)?;

    let no_padding = java_string(env, "NoPadding")?;
    let paddings = string_array(env, &no_padding)?;
    let builder = env
        .call_method(
            &builder,
            "setEncryptionPaddings",
            "([Ljava/lang/String;)Landroid/security/keystore/KeyGenParameterSpec$Builder;",
            &[JValue::Object(AsRef::<JObject>::as_ref(&paddings))],
        )
        .map_err(jerr)?
        .l()
        .map_err(jerr)?;

    let spec = env
        .call_method(
            &builder,
            "build",
            "()Landroid/security/keystore/KeyGenParameterSpec;",
            &[],
        )
        .map_err(jerr)?
        .l()
        .map_err(jerr)?;

    env.call_method(
        &generator,
        "init",
        "(Ljava/security/spec/AlgorithmParameterSpec;)V",
        &[JValue::Object(&spec)],
    )
    .map_err(jerr)?;
    env.call_method(&generator, "generateKey", "()Ljavax/crypto/SecretKey;", &[])
        .map_err(jerr)?;
    Ok(())
}

fn java_string<'local>(env: &mut JNIEnv<'local>, value: &str) -> Result<JObject<'local>> {
    Ok(env.new_string(value).map_err(jerr)?.into())
}

fn string_array<'local>(
    env: &mut JNIEnv<'local>,
    value: &JObject<'local>,
) -> Result<jni::objects::JObjectArray<'local>> {
    let array = env
        .new_object_array(1, "java/lang/String", JObject::null())
        .map_err(jerr)?;
    env.set_object_array_element(&array, 0, value)
        .map_err(jerr)?;
    Ok(array)
}
