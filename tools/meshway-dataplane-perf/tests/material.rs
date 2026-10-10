#![cfg(unix)]
use meshway_dataplane_perf::material;
use std::os::unix::fs::PermissionsExt;

#[test]
fn node_private_file_is_exclusive_and_reconstructs_public_inventory() {
    let path = std::path::Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("../../target")
        .join(format!(
            "material-test-{}",
            meshway_desired_fixture::random_uuid().unwrap()
        ));
    let public = material::generate(&path, "127.0.0.1:41000".parse().unwrap()).unwrap();
    assert_eq!(
        std::fs::metadata(&path).unwrap().permissions().mode() & 0o777,
        0o700
    );
    let file = path.join("seeds.json");
    assert_eq!(
        std::fs::metadata(&file).unwrap().permissions().mode() & 0o777,
        0o600
    );
    let (id, reconstructed) = material::load(&file).unwrap();
    assert_eq!(id, public.id);
    assert_eq!(reconstructed.trusted(), &public.trusted);
    assert!(material::generate(&path, public.socket).is_err());
    std::fs::set_permissions(&file, std::fs::Permissions::from_mode(0o644)).unwrap();
    assert!(material::load(&file).is_err());
    std::fs::remove_dir_all(path).unwrap();
}
