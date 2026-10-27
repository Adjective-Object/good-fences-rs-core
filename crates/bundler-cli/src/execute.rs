use eyre::WrapErr;
use std::io::Write;

use bundler::{
    build::{BuildSession, CssLoad, Entry},
    publish::Cancellation,
};

use crate::invocation::Invocation;

pub(crate) fn build(invocation: &Invocation, stdout: &mut impl Write) -> eyre::Result<()> {
    let start = std::time::Instant::now();
    tracing::info!(target: "bundler::progress", "Loading build configuration");
    let mut options = invocation
        .config
        .load()
        .wrap_err("loading build configuration")?
        .into_options(&invocation.root)
        .wrap_err("translating CLI configuration to build options")?;
    if invocation.css_auto_mount {
        options.css.load = CssLoad::AutoMount;
    }
    if let Some(locale) = invocation
        .locale
        .as_ref()
        .filter(|locale| locale.as_str() != "default")
    {
        options.locale.default = locale.clone();
        options.locale.validate().wrap_err("validating --locale")?;
    }
    for path in &invocation.entries {
        let name = path
            .to_str()
            .ok_or_else(|| eyre::eyre!("entry must be UTF-8"))?
            .to_owned();
        options.entries.insert(
            name,
            Entry {
                path: path.clone(),
                output_name: None,
                namespace: true,
            },
        );
    }
    let (build, published) = BuildSession::new(&invocation.root)?
        .build_and_publish(&options, &invocation.output, &Cancellation::default())
        .wrap_err_with(|| {
            format!(
                "building {} and publishing to {}",
                invocation.root.display(),
                invocation.output.display()
            )
        })?;
    crate::report::write(stdout, &build, &published).wrap_err("writing build report to stdout")?;
    tracing::info!(
        target: "bundler::progress",
        elapsed_ms = start.elapsed().as_millis() as u64,
        files_written = published.files_written,
        changed = published.changed,
        "Build complete"
    );
    Ok(())
}
