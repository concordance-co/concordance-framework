use argon2::{
    password_hash::{rand_core::OsRng as ArgonOsRng, SaltString},
    Argon2, PasswordHash, PasswordHasher, PasswordVerifier,
};

pub fn hash_and_salt_password(password: &str) -> (String, SaltString) {
    let salt = SaltString::generate(&mut ArgonOsRng);
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &salt)
        .unwrap()
        .to_string();
    (password_hash, salt)
}

pub fn hash_with_salt(password: &str, salt: &str) -> String {
    let argon2 = Argon2::default();
    let password_hash = argon2
        .hash_password(password.as_bytes(), &SaltString::from_b64(salt).unwrap())
        .unwrap()
        .to_string();
    password_hash
}

pub fn verify_password(password: &str, hash: &str) -> bool {
    let Ok(hash) = PasswordHash::new(hash) else {
        return false;
    };
    let argon2 = Argon2::default();
    argon2.verify_password(password.as_bytes(), &hash).is_ok()
}
