use std::collections::HashSet;
use std::fs;
use std::path::PathBuf;

fn main() {
    let root = PathBuf::from(std::env::var_os("CARGO_MANIFEST_DIR").expect("manifest directory"))
        .join("../../assets/icons");
    println!("cargo:rerun-if-changed={}", root.display());
    let mut seen = HashSet::new();
    let entries = fs::read_dir(&root).expect("Iconify SVG directory must exist");
    let mut count = 0;
    for entry in entries {
        let path = entry.expect("reading icon directory entry").path();
        if path.extension().and_then(|extension| extension.to_str()) != Some("svg") {
            continue;
        }
        let contents = fs::read_to_string(&path).expect("SVG icon must be readable UTF-8");
        let document = roxmltree::Document::parse(&contents)
            .unwrap_or_else(|error| panic!("{} is malformed SVG: {error}", path.display()));
        let root = document.root_element();
        assert_eq!(
            root.tag_name().name(),
            "svg",
            "{} is not an SVG document",
            path.display()
        );
        assert!(
            root.attribute("viewBox")
                .or_else(|| root.attribute("viewbox"))
                .is_some(),
            "{} has no viewBox",
            path.display()
        );
        for node in document.descendants().filter(roxmltree::Node::is_element) {
            let name = node.tag_name().name();
            assert!(
                !matches!(
                    name.to_ascii_lowercase().as_str(),
                    "script" | "foreignobject"
                ),
                "{} contains an active SVG element",
                path.display()
            );
            for attribute in node.attributes() {
                let attribute_name = attribute.name().to_ascii_lowercase();
                let value = attribute.value().to_ascii_lowercase();
                assert!(
                    attribute_name != "href" && attribute_name != "xlink:href",
                    "{} contains an external SVG reference",
                    path.display()
                );
                assert!(
                    !value.contains("javascript:")
                        && !value.contains("url(http")
                        && !value.contains("url(https"),
                    "{} contains an external or active SVG resource",
                    path.display()
                );
            }
        }
        assert!(
            seen.insert(contents),
            "duplicate SVG asset: {}",
            path.display()
        );
        count += 1;
    }
    assert!(
        count >= 20,
        "expected the bundled Iconify icon set, found {count} SVG files"
    );
}
