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
                "aria2": {
                    "enabled": cfg.aria2.enabled,
                    "executable_path": cfg.aria2.executable_path,
                    "rpc_port": cfg.aria2.rpc_port,
                    "has_secret": cfg.aria2.has_secret(),
                    // rpc_secret intentionally omitted
                },
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
        println!("  aria2");
        println!("    Enabled:      {}", cfg.aria2.enabled);
        println!(
            "    Executable:   {}",
            cfg.aria2
                .executable_path
                .as_deref()
                .map(|p| p.to_string_lossy().into_owned())
                .unwrap_or_else(|| "(PATH discovery)".to_owned())
        );
        println!("    RPC port:     {}", cfg.aria2.rpc_port);
        println!(
            "    RPC secret:   {}",
            if cfg.aria2.has_secret() {
                "[configured]"
            } else {
                "[not set]"
            }
        );
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
