//! The installer against the real servers: downloads the components named in
//! `SLOPSHOP_AI_INSTALL` (comma-separated ids) into the folder `SLOPSHOP_AI_INSTALL_ROOT`.
//! Skipped otherwise (it downloads up to a gigabyte).

use std::path::Path;

use slopshop_ai::install;

#[test]
fn installs_the_named_components() {
    let (Ok(ids), Some(root)) = (
        std::env::var("SLOPSHOP_AI_INSTALL"),
        std::env::var_os("SLOPSHOP_AI_INSTALL_ROOT"),
    ) else {
        eprintln!("SLOPSHOP_AI_INSTALL / SLOPSHOP_AI_INSTALL_ROOT not set: skipped");
        return;
    };
    let root = Path::new(&root);
    for id in ids.split(',') {
        let component = install::component(id).unwrap_or_else(|| panic!("no component {id}"));
        let start = std::time::Instant::now();
        let mut shown = 0;
        install::install(component, root, &mut |p| {
            let percent = p.done * 100 / p.total.max(1);
            if percent >= shown + 10 {
                shown = percent;
                eprintln!("{id}: {percent}%");
            }
            true
        })
        .unwrap_or_else(|e| panic!("{id}: {e}"));
        eprintln!(
            "{id}: {} MB in {:?}",
            component.download_size() >> 20,
            start.elapsed()
        );
        assert!(component.is_installed(root));
    }
}
