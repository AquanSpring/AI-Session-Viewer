use std::fs;
use std::path::Path;

const DEFAULT_PROVIDER: &str = "openai";

pub fn read_current_provider(config_path: &Path) -> (String, bool) {
    let text = match fs::read_to_string(config_path) {
        Ok(s) => s,
        Err(_) => return (DEFAULT_PROVIDER.to_string(), true),
    };
    match parse_root_provider(&text) {
        Some(p) => (p, false),
        None => (DEFAULT_PROVIDER.to_string(), true),
    }
}

fn parse_root_provider(text: &str) -> Option<String> {
    for raw in text.lines() {
        let line = strip_comment(raw).trim();
        if line.is_empty() {
            continue;
        }
        if line.starts_with('[') {
            return None;
        }
        if let Some(rest) = line.strip_prefix("model_provider") {
            let rest = rest.trim_start();
            let Some(rest) = rest.strip_prefix('=') else {
                continue;
            };
            let value = strip_comment(rest).trim();
            let unquoted = value.trim_matches('"').trim_matches('\'').to_string();
            if !unquoted.is_empty() {
                return Some(unquoted);
            }
        }
    }
    None
}

fn strip_comment(s: &str) -> &str {
    let bytes = s.as_bytes();
    let mut in_double = false;
    let mut in_single = false;
    for (i, &b) in bytes.iter().enumerate() {
        match b {
            b'"' if !in_single => in_double = !in_double,
            b'\'' if !in_double => in_single = !in_single,
            b'#' if !in_double && !in_single => return &s[..i],
            _ => {}
        }
    }
    s
}

pub fn list_configured_providers(config_path: &Path) -> Vec<String> {
    let text = match fs::read_to_string(config_path) {
        Ok(s) => s,
        Err(_) => return Vec::new(),
    };
    let mut out = Vec::new();
    for line in text.lines() {
        let line = line.trim();
        let Some(rest) = line.strip_prefix("[model_providers.") else {
            continue;
        };
        let Some(end) = rest.find(']') else { continue };
        let id = rest[..end].trim().trim_matches('"').to_string();
        if !id.is_empty() && !out.contains(&id) {
            out.push(id);
        }
    }
    out
}

pub fn set_root_provider(config_path: &Path, new_provider: &str) -> Result<String, String> {
    let original = fs::read_to_string(config_path).unwrap_or_default();

    let mut lines: Vec<String> = if original.is_empty() {
        Vec::new()
    } else {
        original.split('\n').map(|s| s.to_string()).collect()
    };

    let mut replaced = false;
    let mut first_section_idx: Option<usize> = None;
    for (i, raw) in lines.iter().enumerate() {
        let line = strip_comment(raw).trim();
        if line.starts_with('[') {
            first_section_idx = Some(i);
            break;
        }
        if line.starts_with("model_provider") && line.contains('=') {
            lines[i] = format!("model_provider = \"{}\"", new_provider);
            replaced = true;
            break;
        }
    }

    if !replaced {
        let insert_at = first_section_idx.unwrap_or(lines.len());
        lines.insert(insert_at, format!("model_provider = \"{}\"", new_provider));
        if first_section_idx.is_some() {
            lines.insert(insert_at + 1, String::new());
        }
    }

    let new_text = lines.join("\n");
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    fs::write(config_path, &new_text).map_err(|e| e.to_string())?;
    Ok(original)
}

/// Provider ids built into Codex; sessions saved under them always resolve.
pub const BUILTIN_PROVIDERS: &[&str] = &["openai", "ollama", "lmstudio"];

pub fn is_builtin_provider(id: &str) -> bool {
    BUILTIN_PROVIDERS.contains(&id)
}

fn parse_config(text: &str) -> Result<toml::Table, String> {
    text.parse::<toml::Table>()
        .map_err(|e| format!("config.toml 无法解析：{}", e.message()))
}

/// `[model_providers.*]` ids as Codex sees them (quoted keys, dotted tables,
/// inline tables). Errors when the file is not valid TOML; a missing file
/// defines no providers.
pub fn parse_configured_providers(config_path: &Path) -> Result<Vec<String>, String> {
    let text = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(Vec::new()),
        Err(e) => return Err(format!("读取 config.toml 失败：{e}")),
    };
    let table = parse_config(&text)?;
    Ok(table
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .map(|providers| providers.keys().cloned().collect())
        .unwrap_or_default())
}

fn toml_key(id: &str) -> String {
    if !id.is_empty()
        && id
            .chars()
            .all(|c| c.is_ascii_alphanumeric() || c == '_' || c == '-')
    {
        id.to_string()
    } else {
        toml::Value::String(id.to_string()).to_string()
    }
}

/// Definition appended for one missing provider id. Without `template` it
/// reuses the current OpenAI login and the default endpoint (no `base_url`),
/// otherwise it copies the settings of the configured provider `template`.
fn compat_block(id: &str, template: Option<(&str, &toml::Table)>) -> String {
    let mut out = format!(
        "\n# Added by AI Session Viewer: keep conversations saved under provider {} resumable.\n",
        toml::Value::String(id.to_string())
    );
    match template {
        None => {
            out.push_str("# Uses the current OpenAI login and default endpoint.\n");
            out.push_str(&format!("[model_providers.{}]\n", toml_key(id)));
            out.push_str(&format!(
                "name = {}\n",
                toml::Value::String(format!("OpenAI (legacy {id} compatibility)"))
            ));
            out.push_str("wire_api = \"responses\"\n");
            out.push_str("requires_openai_auth = true\n");
        }
        Some((source, settings)) => {
            out.push_str(&format!(
                "# Copied from [model_providers.{}].\n",
                toml_key(source)
            ));
            out.push_str(&format!("[model_providers.{}]\n", toml_key(id)));
            for (key, value) in settings {
                out.push_str(&format!("{} = {}\n", toml_key(key), value));
            }
        }
    }
    out
}

/// Append compatibility definitions for `ids` to `config.toml`, leaving every
/// existing line untouched. The result is parsed before it is written, and
/// the write is aborted if the file changed in the meantime. Returns the
/// original text.
pub fn append_compat_providers(
    config_path: &Path,
    ids: &[String],
    template: Option<&str>,
) -> Result<String, String> {
    let original = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读取 config.toml 失败：{e}")),
    };
    let table = parse_config(&original)?;
    let providers = table.get("model_providers");
    if providers.is_some_and(|value| !value.is_table()) {
        return Err("config.toml 中的 model_providers 不是表，无法追加定义".to_string());
    }
    let configured = providers.and_then(toml::Value::as_table);
    let template = match template {
        Some(source) => {
            let settings = configured
                .and_then(|providers| providers.get(source))
                .and_then(toml::Value::as_table)
                .ok_or_else(|| format!("config.toml 中没有 provider「{source}」"))?;
            Some((source, settings))
        }
        None => None,
    };

    let mut next = original.clone();
    if !next.is_empty() && !next.ends_with('\n') {
        next.push('\n');
    }
    for id in ids {
        if configured.is_some_and(|providers| providers.contains_key(id)) {
            return Err(format!("provider「{id}」已经定义，无需修复"));
        }
        next.push_str(&compat_block(id, template));
    }

    let parsed = parse_config(&next).map_err(|e| format!("生成的配置无效，未写入：{e}"))?;
    let defined = parsed
        .get("model_providers")
        .and_then(toml::Value::as_table)
        .is_some_and(|providers| ids.iter().all(|id| providers.contains_key(id)));
    if !defined {
        return Err("生成的配置缺少新的 provider 定义，未写入".to_string());
    }

    let current = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => String::new(),
        Err(e) => return Err(format!("读取 config.toml 失败：{e}")),
    };
    if current != original {
        return Err("config.toml 在修复期间被其他程序修改，已停止写入，请刷新后重试".to_string());
    }
    if let Some(parent) = config_path.parent() {
        fs::create_dir_all(parent).map_err(|e| e.to_string())?;
    }
    let tmp = config_path.with_extension("toml.asv-tmp");
    fs::write(&tmp, &next).map_err(|e| format!("写入 config.toml 失败：{e}"))?;
    if let Err(e) = fs::rename(&tmp, config_path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("保存 config.toml 失败：{e}"));
    }
    Ok(original)
}

/// Re-key the per-directory `[projects."<path>"]` entries (trust levels)
/// through `remap`, keeping their formatting and position. An entry whose new
/// path already has one is left alone. With `dry_run` nothing is written.
/// Returns how many entries were (or would be) re-keyed.
pub fn remap_project_entries(
    config_path: &Path,
    remap: impl Fn(&str) -> Option<String>,
    dry_run: bool,
) -> Result<u32, String> {
    let text = match fs::read_to_string(config_path) {
        Ok(text) => text,
        Err(e) if e.kind() == std::io::ErrorKind::NotFound => return Ok(0),
        Err(e) => return Err(format!("读取 config.toml 失败：{e}")),
    };
    let mut doc = text
        .parse::<toml_edit::DocumentMut>()
        .map_err(|e| format!("config.toml 无法解析：{e}"))?;
    let Some(projects) = doc
        .get_mut("projects")
        .and_then(|item| item.as_table_like_mut())
    else {
        return Ok(0);
    };
    let keys: Vec<String> = projects.iter().map(|(key, _)| key.to_string()).collect();
    let mut moved = 0u32;
    for key in keys {
        let Some(next) = remap(&key).filter(|next| *next != key) else {
            continue;
        };
        if projects.contains_key(&next) {
            continue;
        }
        if let Some(item) = projects.remove(&key) {
            projects.insert(&next, item);
            moved += 1;
        }
    }
    if dry_run || moved == 0 {
        return Ok(moved);
    }
    let next = doc.to_string();
    parse_config(&next).map_err(|e| format!("生成的配置无效，未写入：{e}"))?;
    let tmp = config_path.with_extension("toml.asv-tmp");
    fs::write(&tmp, &next).map_err(|e| format!("写入 config.toml 失败：{e}"))?;
    if let Err(e) = fs::rename(&tmp, config_path) {
        let _ = fs::remove_file(&tmp);
        return Err(format!("保存 config.toml 失败：{e}"));
    }
    Ok(moved)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn rekeys_project_trust_entries() {
        let path = temp_config(
            "model = \"x\"\n\n[projects.\"/old/app\"]\ntrust_level = \"trusted\"\n\n[projects.\"/other\"]\ntrust_level = \"trusted\"\n",
        );
        let remap = |value: &str| {
            crate::transfer::remap_path(value, "/old", "/new")
        };
        assert_eq!(remap_project_entries(&path, remap, true).unwrap(), 1);
        assert_eq!(remap_project_entries(&path, remap, false).unwrap(), 1);
        let table = fs::read_to_string(&path)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        assert_eq!(table["projects"]["/new/app"]["trust_level"].as_str(), Some("trusted"));
        assert!(table["projects"].get("/old/app").is_none());
        assert!(table["projects"].get("/other").is_some());
    }

    fn temp_config(text: &str) -> std::path::PathBuf {
        let dir = std::env::temp_dir().join(format!("asv-config-{}", uuid::Uuid::new_v4()));
        fs::create_dir_all(&dir).unwrap();
        let path = dir.join("config.toml");
        fs::write(&path, text).unwrap();
        path
    }

    #[test]
    fn appends_openai_compat_definition_without_touching_existing_lines() {
        let original = "model = \"gpt-5\"\n\n[projects.\"/work\"]\ntrust_level = \"trusted\"\n";
        let path = temp_config(original);
        append_compat_providers(&path, &["custom".to_string()], None).unwrap();
        let text = fs::read_to_string(&path).unwrap();
        assert!(text.starts_with(original));
        assert_eq!(parse_configured_providers(&path).unwrap(), vec!["custom".to_string()]);
        let table = text.parse::<toml::Table>().unwrap();
        let custom = &table["model_providers"]["custom"];
        assert_eq!(custom["requires_openai_auth"].as_bool(), Some(true));
        assert_eq!(custom["wire_api"].as_str(), Some("responses"));
        assert!(custom.get("base_url").is_none());
        assert_eq!(table["model"].as_str(), Some("gpt-5"));
    }

    #[test]
    fn copies_template_provider_and_quotes_odd_ids() {
        let path = temp_config(
            "[model_providers.relay]\nname = \"Relay\"\nbase_url = \"https://relay.example/v1\"\nhttp_headers = { \"X-A\" = \"1\" }\n",
        );
        append_compat_providers(&path, &["old relay".to_string()], Some("relay")).unwrap();
        let table = fs::read_to_string(&path)
            .unwrap()
            .parse::<toml::Table>()
            .unwrap();
        let copied = &table["model_providers"]["old relay"];
        assert_eq!(copied["base_url"].as_str(), Some("https://relay.example/v1"));
        assert_eq!(copied["http_headers"]["X-A"].as_str(), Some("1"));
    }

    #[test]
    fn refuses_existing_or_non_table_definitions() {
        let path = temp_config("[model_providers.custom]\nname = \"x\"\n");
        assert!(append_compat_providers(&path, &["custom".to_string()], None).is_err());
        let path = temp_config("model_providers = 1\n");
        assert!(append_compat_providers(&path, &["custom".to_string()], None).is_err());
    }
}
