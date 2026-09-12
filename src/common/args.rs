use super::*;

#[allow(dead_code)]
#[inline]
fn arg_name(name: &str) -> String {
    name.to_uppercase().replace('_', "-")
}

#[allow(dead_code)]
#[inline]
pub fn set_arg(name: &str, value: &str) {
    std::env::set_var(arg_name(name), value);
}

#[allow(dead_code)]
pub fn init_args(args: &str, name: &str, about: &str) {
    let matches = App::new(name)
        .version(crate::version::VERSION)
        .author("Purslane Ltd. <info@rustdesk.com>")
        .about(about)
        .args_from_usage(args)
        .get_matches();
    if let Ok(v) = Ini::load_from_file(".env") {
        if let Some(section) = v.section(None::<String>) {
            section
                .iter()
                .for_each(|(k, v)| set_arg(k, v));
        }
    }
    if let Some(config) = matches.value_of("config") {
        if let Ok(v) = Ini::load_from_file(config) {
            if let Some(section) = v.section(None::<String>) {
                section
                    .iter()
                    .for_each(|(k, v)| set_arg(k, v));
            }
        }
    }
    for (k, v) in matches.args {
        if let Some(v) = v.vals.first() {
            set_arg(k, &v.to_string_lossy());
        }
    }
}

#[allow(dead_code)]
pub fn get_arg_opt(name: &str) -> Option<String> {
    let dashed = arg_name(name);
    let underscored = dashed.replace('-', "_");
    let lower_dashed = dashed.to_lowercase();
    let lower_underscored = underscored.to_lowercase();
    for alias in [&dashed, &underscored, &lower_dashed, &lower_underscored] {
        if let Ok(value) = std::env::var(alias) {
            return Some(value);
        }
    }
    let mut aliases = std::env::vars_os()
        .filter_map(|(key, value)| {
            let key = key.into_string().ok()?;
            if arg_name(&key) == dashed {
                Some((key, value.into_string().ok()?))
            } else {
                None
            }
        })
        .collect::<Vec<_>>();
    aliases.sort_by(|a, b| a.0.cmp(&b.0));
    aliases.into_iter().next().map(|(_, value)| value)
}

#[allow(dead_code)]
#[inline]
pub fn get_arg(name: &str) -> String {
    get_arg_or(name, "".to_owned())
}

#[allow(dead_code)]
#[inline]
pub fn get_arg_or(name: &str, default: String) -> String {
    get_arg_opt(name).unwrap_or(default)
}
