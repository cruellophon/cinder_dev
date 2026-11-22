pub mod routes;
pub mod state;

use crate::routes::build_router;
use crate::state::{AppCache, AppState, STRICT_CSP};
use axum::{
    extract::Request,
    http::{header, HeaderValue},
    middleware::{self, Next},
    response::Response,
};
use harness_models::{DownloadManager, ServerManager};
use harness_net::NetworkGate;
use harness_store::{BumboPaths, BumboStore};
use std::sync::Arc;

pub fn run_cinder_entry() -> anyhow::Result<()> {
    let _ = tracing_subscriber::fmt()
        .with_env_filter(
            tracing_subscriber::EnvFilter::try_from_default_env()
                .unwrap_or_else(|_| "bumbo_app=info,cinder_gui=info".into()),
        )
        .try_init();

    let args: Vec<String> = std::env::args().collect();
    let headless = args.iter().any(|a| a == "--headless");

    if headless {
        let rt = tokio::runtime::Builder::new_multi_thread()
            .enable_all()
            .build()?;
        return rt.block_on(run_headless_server(&args));
    }

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

pub async fn run_headless_server(args: &[String]) -> anyhow::Result<()> {
    let mut requested_port: u16 = 17770;
    for pair in args.windows(2) {
        if pair[0] == "--port" {
            if let Ok(p) = pair[1].parse::<u16>() {
                requested_port = p;
            }
        }
    }

    let paths = BumboPaths::discover();
    let store = Arc::new(BumboStore::init(paths.clone())?);
    let initial_privacy = store.snapshot().await.settings.privacy_mode;
    let gate = Arc::new(NetworkGate::new(initial_privacy)?);
    let server_mgr = Arc::new(ServerManager::new(paths.engines_dir.clone()));
    let download_mgr = Arc::new(DownloadManager::new());

    {
        let store_bg = Arc::clone(&store);
        let srv_bg = Arc::clone(&server_mgr);
        let gate_bg = Arc::clone(&gate);
        tokio::spawn(async move {
            let mut interval = tokio::time::interval(std::time::Duration::from_secs(60));
            loop {
                interval.tick().await;
                let mins = store_bg.snapshot().await.settings.idle_unload_minutes;
                if srv_bg.check_idle_unload(&gate_bg, mins).await {
                    tracing::info!("Auto-unloaded idle local model after {mins}m to free VRAM");
                }
            }
        });
    }

    let state = AppState {
        gate,
        store,
        server_mgr,
        download_mgr,
        install_jobs: harness_tools::InstallJobManager::new(),
        cache: Arc::new(AppCache::default()),
    };

    let app = build_router(state).layer(middleware::from_fn(csp_middleware));

    let listener = match tokio::net::TcpListener::bind(("127.0.0.1", requested_port)).await {
        Ok(l) => l,
        Err(_) => tokio::net::TcpListener::bind(("127.0.0.1", 0)).await?,
    };
    let port = listener.local_addr()?.port();
    let url = format!("http://127.0.0.1:{port}");
    println!("Cinder Headless Server running at {url}");

    axum::serve(listener, app).await?;
    Ok(())
}

async fn csp_middleware(req: Request, next: Next) -> Response {
    let mut resp = next.run(req).await;
    resp.headers_mut().insert(
        header::CONTENT_SECURITY_POLICY,
        HeaderValue::from_static(STRICT_CSP),
    );
    resp.headers_mut().insert(
        header::X_CONTENT_TYPE_OPTIONS,
        HeaderValue::from_static("nosniff"),
    );
    resp
}
