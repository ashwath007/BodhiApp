#[cfg(feature = "native")]
use crate::native;

use crate::{
  convert::{
    build_create_command, build_list_command, build_manage_alias_command, build_pull_command,
    build_run_command, build_serve_command,
  },
  error::{BodhiError, Result},
};
use clap::Parser;
use commands::{Cli, Command, DefaultStdoutWriter, EnvCommand};
use objs::FluentLocalizationService;
use services::{
  db::{DbPool, DbService, DefaultTimeService, SqliteDbService},
  hash_key, DefaultAppService, DefaultSecretService, HfHubService, KeycloakAuthService,
  KeyringStore, LocalDataService, MokaCacheService, SettingService, SqliteSessionService,
  SystemKeyringStore,
};
use std::{env, sync::Arc};
use tokio::runtime::Builder;

const SECRET_KEY: &str = "secret_key";

pub fn main_internal(setting_service: Arc<dyn SettingService>) -> Result<()> {
  let runtime = Builder::new_multi_thread().enable_all().build()?;
  runtime.block_on(async move { aexecute(setting_service).await })
}

async fn aexecute(setting_service: Arc<dyn SettingService>) -> Result<()> {
  let bodhi_home = setting_service.bodhi_home();
  let hf_cache = setting_service.hf_cache();
  let hub_service = Arc::new(HfHubService::new_from_hf_cache(hf_cache, true));
  let data_service = LocalDataService::new(bodhi_home.clone(), hub_service.clone());
  let app_suffix = if setting_service.is_production() {
    ""
  } else {
    " - Dev"
  };
  let app_name = format!("Bodhi App{app_suffix}");
  let secrets_path = setting_service.secrets_path();
  let encryption_key = setting_service.encryption_key();
  let encryption_key = encryption_key
    .map(|key| Ok(hash_key(&key)))
    .unwrap_or_else(|| SystemKeyringStore::new(&app_name).get_or_generate(SECRET_KEY))?;

  let secret_service = DefaultSecretService::new(encryption_key, &secrets_path)?;

  let app_db_pool = DbPool::connect(&format!(
    "sqlite:{}",
    setting_service.app_db_path().display()
  ))
  .await?;
  let time_service = Arc::new(DefaultTimeService);
  let db_service = SqliteDbService::new(app_db_pool, time_service.clone());
  db_service.migrate().await?;

  let session_db_pool = DbPool::connect(&format!(
    "sqlite:{}",
    setting_service.session_db_path().display()
  ))
  .await?;
  let session_service = SqliteSessionService::new(session_db_pool);
  session_service.migrate().await?;

  let cache_service = MokaCacheService::default();

  let auth_url = setting_service.auth_url();
  let auth_realm = setting_service.auth_realm();
  let auth_service = KeycloakAuthService::new(&setting_service.version(), auth_url, auth_realm);
  let localization_service = FluentLocalizationService::get_instance();
  localization_service
    .load_resource(objs::l10n::L10N_RESOURCES)?
    .load_resource(objs::gguf::l10n::L10N_RESOURCES)?
    .load_resource(llama_server_proc::l10n::L10N_RESOURCES)?
    .load_resource(services::l10n::L10N_RESOURCES)?
    .load_resource(commands::l10n::L10N_RESOURCES)?
    .load_resource(server_core::l10n::L10N_RESOURCES)?
    .load_resource(auth_middleware::l10n::L10N_RESOURCES)?
    .load_resource(routes_oai::l10n::L10N_RESOURCES)?
    .load_resource(routes_app::l10n::L10N_RESOURCES)?
    .load_resource(routes_all::l10n::L10N_RESOURCES)?
    .load_resource(server_app::l10n::L10N_RESOURCES)?
    .load_resource(crate::l10n::L10N_RESOURCES)?;

  let app_service = DefaultAppService::new(
    setting_service.clone(),
    hub_service,
    Arc::new(data_service),
    Arc::new(auth_service),
    Arc::new(db_service),
    Arc::new(session_service),
    Arc::new(secret_service),
    Arc::new(cache_service),
    localization_service,
    time_service,
  );
  let service = Arc::new(app_service);

  let args = env::args().collect::<Vec<_>>();
  if args.len() == 1 && setting_service.is_native() {
    if cfg!(feature = "native") {
      // the app was launched executing the executable, launch the native app with system tray
      #[cfg(feature = "native")]
      native::NativeCommand::new(service.clone(), true)
        .aexecute(Some(crate::ui::router()))
        .await?;
    } else {
      Err(BodhiError::Unreachable(
        r#"setting_service.is_native() returned true, but cfg!(feature = "native") is false"#
          .to_string(),
      ))?;
    }
  }

  // the app was called from wrapper
  // or the executable was called from outside the `Bodhi.app` bundle
  let cli = Cli::parse();
  match cli.command {
    Command::Envs {} => {
      EnvCommand::new(service).execute()?;
    }
    Command::App { ui: _ui } => {
      if setting_service.is_native() {
        if cfg!(feature = "native") {
          #[cfg(feature = "native")]
          native::NativeCommand::new(service, _ui)
            .aexecute(Some(crate::ui::router()))
            .await?;
        } else {
          Err(BodhiError::Unreachable(
            r#"setting_service.is_native() returned true, but cfg!(feature = "native") is false"#
              .to_string(),
          ))?;
        }
      } else {
        Err(BodhiError::NativeNotSupported)?;
      }
    }
    Command::Serve { host, port } => {
      let serve_command = build_serve_command(host, port)?;
      serve_command
        .aexecute(service, Some(crate::ui::router()))
        .await?;
    }
    Command::List { remote, models } => {
      let list_command = build_list_command(remote, models)?;
      list_command.execute(service)?;
    }
    Command::Pull {
      alias,
      repo,
      filename,
      snapshot,
    } => {
      let pull_command = build_pull_command(alias, repo, filename, snapshot)?;
      pull_command.execute(service)?;
    }
    cmd @ Command::Create { .. } => {
      let create_command = build_create_command(cmd)?;
      create_command.execute(service)?;
    }
    Command::Run { alias } => {
      let run_command = build_run_command(alias)?;
      run_command.aexecute(service).await?;
    }
    cmd @ (Command::Show { .. }
    | Command::Cp { .. }
    | Command::Edit { .. }
    | Command::Rm { .. }) => {
      let manage_alias_command = build_manage_alias_command(cmd)?;
      manage_alias_command.execute(service, &mut DefaultStdoutWriter::default())?;
    }
    #[cfg(debug_assertions)]
    Command::Secrets {} => {
      use services::AppService;
      println!("{}", service.secret_service().dump()?);
    }
  }
  Ok(())
}
