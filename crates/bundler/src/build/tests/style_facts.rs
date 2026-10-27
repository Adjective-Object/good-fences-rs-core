use super::*;

#[test]
fn stylesheet_preparation_retains_mapped_facts_and_sass_snapshots_not_original_buffers() {
    for cache in [
        CacheMode::ReadWrite,
        CacheMode::ReadOnly,
        CacheMode::Disabled,
    ] {
        for source_maps in [false, true] {
            let directory = tempfile::tempdir_in(".").unwrap();
            let root = directory.path().canonicalize().unwrap();
            let plain = ".button {background: url('./icon.svg'); color: red}";
            let sass =
                "@use './palette'; .button {color: palette.$color; background: url('./icon.svg')}";
            write(
                &root,
                "main.mjs",
                "import './plain.module.css'; import './style.module.scss';",
            );
            write(&root, "plain.module.css", plain);
            write(&root, "style.module.scss", sass);
            write(&root, "_palette.scss", "$color: red;");
            write(&root, "icon.svg", "<svg/>");
            let mut config = options();
            config.cache = cache;
            config.output.source_maps = source_maps;
            config.css.css_modules = CssModules::Enabled;
            config.resolver.target_env = TargetEnv::Browser;
            let built = BuildSession::new(&root).unwrap().build(&config).unwrap();
            let mut artifacts = built.product.artifacts.clone();
            let expected = artifacts.resources.clone();
            artifacts.resources.clear();
            let inventory = Inventory::new(&root);
            let plain_path = inventory.admit(&root.join("plain.module.css")).unwrap();
            let sass_path = inventory.admit(&root.join("style.module.scss")).unwrap();
            let base = crate::output_url::PublicBase::new("./").unwrap();
            let resolver = import_resolver::structural::StructuralResolver::new(
                root.clone(),
                config.resolver.clone(),
                BTreeSet::new(),
            );
            let mut styles = crate::css::Styles::new(
                &root,
                &inventory,
                &base,
                &config.css,
                &config.resolver,
                source_maps,
                cache,
            );
            styles
                .discover(
                    &plain_path,
                    plain.as_bytes(),
                    &resolver,
                    &resolver,
                    &resolver,
                )
                .unwrap();
            styles
                .discover(&sass_path, sass.as_bytes(), &resolver, &resolver, &resolver)
                .unwrap();
            styles.finish_discovery().unwrap();
            let (original, facts) = styles.storage(&plain_path);
            assert!(original.generated_backing().is_none());
            assert!(facts.backing().is_mapped());
            let (compiled, facts) = styles.storage(&sass_path);
            assert!(compiled.generated_backing().unwrap().is_mapped());
            assert!(facts.backing().is_mapped());
            let prepared = compiled.text(&sass_path).unwrap();
            assert!(prepared.contains("color: red"));
            assert!(!prepared.contains("@use"));
            let compilations = crate::sass::compilations();
            styles.emit(&plain_path, &mut artifacts).unwrap();
            styles.emit(&sass_path, &mut artifacts).unwrap();
            styles.recheck().unwrap();
            assert_eq!(artifacts.resources, expected);
            assert_eq!(
                crate::sass::compilations(),
                compilations,
                "emission recompiled Sass"
            );

            write(&root, "_palette.scss", "$color: blue;");
            let error = styles.recheck().unwrap_err();
            assert!(format!("{error:#}").contains("_palette.scss"));
            assert!(styles.emit(&sass_path, &mut artifacts).is_err());
            write(&root, "_palette.scss", "$color: red;");
            write(&root, "icon.svg", "<svg>changed</svg>");
            assert!(styles.emit(&plain_path, &mut artifacts).is_err());
            write(&root, "icon.svg", "<svg/>");
            write(&root, "plain.module.css", ".changed {}");
            assert!(styles.storage(&plain_path).0.text(&plain_path).is_err());
            assert!(styles.emit(&plain_path, &mut artifacts).is_err());
            write(&root, "plain.module.css", plain);
            write(&root, "style.module.scss", "@error 'changed original';");
            assert!(styles.storage(&sass_path).0.text(&sass_path).is_err());
            assert!(styles.emit(&sass_path, &mut artifacts).is_err());
            assert_eq!(crate::sass::compilations(), compilations);
        }
    }
}
