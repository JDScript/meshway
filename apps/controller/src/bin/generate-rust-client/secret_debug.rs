//! Uniform opaque Debug for generated HTTP DTOs, including every secret-bearing
//! nested container. Independent of HTTP transport construction/injection.

pub fn adapt(content: &mut String) -> Result<(), Box<dyn std::error::Error>> {
    // Checked source adapter; generated artifacts are never patched by hand.
    if !content.contains("Debug") {
        return Err("upstream DTO derives changed; review secret protection".into());
    }
    *content = content.replace("Debug, ", "").replace(", Debug", "");
    let names: Vec<_> = content
        .lines()
        .filter_map(|line| {
            line.strip_prefix("pub struct ")
                .or_else(|| line.strip_prefix("pub enum "))
                .and_then(|value| value.split_whitespace().next())
                .map(str::to_owned)
        })
        .collect();
    if names.is_empty() {
        return Err("upstream DTO declarations changed; review secret protection".into());
    }
    for name in names {
        content.push_str(&format!("\nimpl std::fmt::Debug for {name} {{ fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {{ f.write_str(\"{name}([REDACTED])\") }} }}\n"));
    }
    Ok(())
}
