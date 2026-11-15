#![cfg_attr(not(debug_assertions), windows_subsystem = "windows")]

use hybrid_app::cli::{
    acquire_single_instance, resolve_user_data_dir, shutdown_temp_cleanup, startup_shell_bootstrap,
    AppCliArgs, SingleInstanceBootstrap,
};
use hybrid_app::renderer::{render_headless_snapshot, run_desktop_app};

fn main() {
    let _tokio_rt = tokio::runtime::Builder::new_multi_thread()
        .enable_all()
        .build()
        .ok();
    let _tokio_guard = _tokio_rt.as_ref().map(|rt| rt.enter());

    let args = match AppCliArgs::parse_env() {
        Ok(a) => a,
        Err(err) => {
            eprintln!("hybrid-app argument error: {err}");
            std::process::exit(2);
        }
    };

    if let Some(snapshot_opts) = &args.headless_snapshot {
        if let Err(err) = render_headless_snapshot(snapshot_opts) {
            eprintln!("headless snapshot error: {err}");
            std::process::exit(1);
        }
        return;
    }

    let _guards = startup_shell_bootstrap(false);
    let single = match acquire_single_instance(&args) {
        Ok(SingleInstanceBootstrap::Primary(server)) => server,
        Ok(SingleInstanceBootstrap::SecondaryForwarded) => {
            shutdown_temp_cleanup();
            return;
        }
        Err(err) => {
            eprintln!("single-instance error: {err}");
            std::process::exit(1);
        }
    };

    let user_data_dir = resolve_user_data_dir(args.app_data_dir.as_deref());
    if let Err(err) = run_desktop_app(args, single, user_data_dir) {
        eprintln!("hybrid-app runtime error: {err}");
        shutdown_temp_cleanup();
        std::process::exit(1);
    }

    shutdown_temp_cleanup();
}
