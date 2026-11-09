use crate::{bootstrap, exit::ExitCode, output};

pub async fn run(json: bool) -> Result<ExitCode, Box<dyn std::error::Error>> {
    let svc = bootstrap::start().await?;
    let cfg = svc.config_service.load().await.unwrap_or_default();

    if json {
        // Never include secrets in output.
        output::print_json_ok(&serde_json::json!({
            "ok": true,
            "config": {
                "version": cfg.version,
                "direct_http": { "enabled": cfg.direct_http.enabled },
                "yt_dlp": {
                    "enabled": cfg.yt_dlp.enabled,
                    "executable_path": cfg.yt_dlp.executable_path,
                }
            }
        }));
    } else {
        println!("Configuration (non-sensitive fields)");
        println!("  Version:        {}", cfg.version);
        println!();
        println!("  Direct HTTP");
        println!("    Enabled:      {}", cfg.direct_http.enabled);
        println!();
        println!("  yt-dlp");
        println!("    Enabled:      {}", cfg.yt_dlp.enabled);
        println!(
            "    Executable:   {}",
            cfg.yt_dlp
                .executable_path
                .as_deref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(PATH discovery)".to_owned())
        );
    }

    svc.shutdown().await?;
    Ok(crate::exit::SUCCESS)
}
