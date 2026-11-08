use std::fs;
use std::path::Path;

const MAX_MANIFEST_BYTES: u64 = 1024 * 1024;

/// Detects common project types by inspecting fixed, shallow manifest paths.
/// It never evaluates project scripts or follows directory trees.
pub fn detect_frameworks(root: &Path) -> Vec<String> {
    let composer = read_manifest(root.join("composer.json"));
    let package = read_manifest(root.join("package.json"));
    let pyproject = read_manifest(root.join("pyproject.toml"));
    let requirements = read_manifest(root.join("requirements.txt"));
    let go_mod = read_manifest(root.join("go.mod"));
    let cargo = read_manifest(root.join("Cargo.toml"));
    let mut frameworks = Vec::new();

    if root.join("artisan").is_file() && composer.contains("laravel/framework") {
        frameworks.push("Laravel".into());
    }
    if root.join("bin/console").is_file() && composer.contains("symfony/framework-bundle") {
        frameworks.push("Symfony".into());
    }
    if root.join("wp-config.php").is_file() || root.join("wp-includes/version.php").is_file() {
        frameworks.push("WordPress".into());
    }
    if package.contains(r#""next""#) || package.contains("'next'") {
        frameworks.push("Next.js".into());
    }
    if package.contains(r#""nuxt""#) || package.contains("'nuxt'") {
        frameworks.push("Nuxt".into());
    }
    if package.contains("@sveltejs/kit") {
        frameworks.push("SvelteKit".into());
    }
    if root.join("manage.py").is_file()
        && (pyproject.to_lowercase().contains("django")
            || requirements.to_lowercase().contains("django"))
    {
        frameworks.push("Django".into());
    }
    if pyproject.to_lowercase().contains("fastapi")
        || requirements.to_lowercase().contains("fastapi")
    {
        frameworks.push("FastAPI".into());
    }
    if go_mod.contains("github.com/gin-gonic/gin") {
        frameworks.push("Gin".into());
    }
    if go_mod.contains("github.com/gofiber/fiber") {
        frameworks.push("Fiber".into());
    }
    if cargo.contains("axum") {
        frameworks.push("Axum".into());
    }
    if cargo.contains("actix-web") {
        frameworks.push("Actix Web".into());
    }
    frameworks
}

fn read_manifest(path: impl AsRef<Path>) -> String {
    let path = path.as_ref();
    let Ok(metadata) = fs::metadata(path) else {
        return String::new();
    };
    if !metadata.is_file() || metadata.len() > MAX_MANIFEST_BYTES {
        return String::new();
    }
    fs::read_to_string(path).unwrap_or_default().to_lowercase()
}
