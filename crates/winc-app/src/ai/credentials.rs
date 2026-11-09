//! 仅在系统凭据存储中保存密钥；以目标接口区分账号，避免修改地址后沿用另一站点的密钥。

use winc_i18n::t;

use super::{endpoint, validate_key};

/// 读取本接口的密钥；用户明确设置的环境变量可作为后备。
pub(crate) fn load_api_key(base_url: &str) -> Result<Option<String>, String> {
    let account = endpoint(base_url)?;
    let stored = load(&account);
    #[cfg(test)]
    let key = stored?;
    #[cfg(not(test))]
    let key = match stored {
        Ok(Some(key)) => Some(key),
        result => match std::env::var("WINC_AI_API_KEY")
            .ok()
            .filter(|v| !v.is_empty())
        {
            Some(key) => Some(key),
            None => result?,
        },
    };
    if let Some(key) = &key {
        validate_key(key)?;
    }
    Ok(key)
}

/// 保存到本接口的系统凭据；空字符串删除，删除不存在的记录也视为成功。
pub(crate) fn save_api_key(base_url: &str, key: &str) -> Result<(), String> {
    let account = endpoint(base_url)?;
    if !key.is_empty() {
        validate_key(key)?;
    }
    save(&account, key)
}

#[cfg(not(test))]
fn entry(account: &str) -> Result<keyring::Entry, String> {
    keyring::Entry::new("Winc AI", account)
        .map_err(|_| t!("ai-client-credential-failed").to_owned())
}

#[cfg(not(test))]
fn load(account: &str) -> Result<Option<String>, String> {
    match entry(account)?.get_password() {
        Ok(key) => Ok(Some(key)),
        Err(keyring::Error::NoEntry) => Ok(None),
        Err(_) => Err(t!("ai-client-credential-failed").to_owned()),
    }
}

#[cfg(not(test))]
fn save(account: &str, key: &str) -> Result<(), String> {
    let entry = entry(account)?;
    let result = if key.is_empty() {
        entry.delete_credential()
    } else {
        entry.set_password(key)
    };
    match result {
        Ok(()) | Err(keyring::Error::NoEntry) if key.is_empty() => Ok(()),
        Ok(()) => Ok(()),
        Err(_) => Err(t!("ai-client-credential-failed").to_owned()),
    }
}

// 测试进程不读写系统凭据，也不读取宿主用户的环境密钥。
#[cfg(test)]
fn store() -> &'static std::sync::Mutex<std::collections::BTreeMap<String, String>> {
    static STORE: std::sync::OnceLock<
        std::sync::Mutex<std::collections::BTreeMap<String, String>>,
    > = std::sync::OnceLock::new();
    STORE.get_or_init(Default::default)
}

#[cfg(test)]
fn load(account: &str) -> Result<Option<String>, String> {
    store()
        .lock()
        .map(|s| s.get(account).cloned())
        .map_err(|_| t!("ai-client-credential-failed").to_owned())
}

#[cfg(test)]
fn save(account: &str, key: &str) -> Result<(), String> {
    let mut store = store()
        .lock()
        .map_err(|_| t!("ai-client-credential-failed").to_owned())?;
    if key.is_empty() {
        store.remove(account);
    } else {
        store.insert(account.to_owned(), key.to_owned());
    }
    Ok(())
}
