use std::{fs, path::PathBuf};

fn main() -> anyhow::Result<()> {
    let mut args = std::env::args_os().skip(1);
    let Some(output) = args.next().map(PathBuf::from) else {
        anyhow::bail!("usage: export-openapi <output>");
    };
    if output.as_os_str().is_empty() || args.next().is_some() {
        anyhow::bail!("usage: export-openapi <output>");
    }
    if let Some(parent) = output.parent().filter(|path| !path.as_os_str().is_empty()) {
        fs::create_dir_all(parent)?;
    }
    fs::write(
        output,
        serde_json::to_string_pretty(&context69::api::openapi_document())?,
    )?;
    Ok(())
}
