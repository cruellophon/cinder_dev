pub fn run_cinder_entry() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "cinder_gui=info".into()),
        )
        .try_init();

    let args: Vec<String> = std::env::args().collect();
    let open_stage = args.iter().any(|a| a == "--open-stage");
    let open_settings = args.iter().any(|a| a == "--open-settings");
    let open_forge = args.iter().any(|a| a == "--open-forge");
    let open_customizations = args.iter().any(|a| a == "--open-customizations");
    let mut screenshot_out: Option<std::path::PathBuf> = None;
    for pair in args.windows(2) {
        if pair[0] == "--screenshot" {
            screenshot_out = Some(std::path::PathBuf::from(&pair[1]));
        }
    }

    cinder_gui::run_native_cinder_with_launch(cinder_gui::CinderLaunchOptions {
        screenshot_out,
        open_stage,
        open_settings,
        open_forge,
        open_customizations,
    })
}
