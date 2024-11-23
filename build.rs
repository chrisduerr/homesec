use pkg_config::Config;

fn main() {
    Config::new().atleast_version("2.7.5").probe("libcryptsetup").unwrap();
}
