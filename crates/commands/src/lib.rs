#[cfg(feature = "test-utils")]
pub mod test_utils;
#[cfg(all(not(feature = "test-utils"), test))]
pub mod test_utils;

mod cmd_alias;
mod cmd_cli;
mod cmd_create;
mod cmd_envs;
mod cmd_list;
mod cmd_pull;
pub mod objs_ext;
mod out_writer;

pub use cmd_alias::*;
pub use cmd_cli::*;
pub use cmd_create::*;
pub use cmd_envs::*;
pub use cmd_list::*;
pub use cmd_pull::*;
pub use out_writer::*;

pub mod l10n {
  use include_dir::Dir;

  pub const L10N_RESOURCES: &Dir = &include_dir::include_dir!("$CARGO_MANIFEST_DIR/src/resources");
}
