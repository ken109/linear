fn main() {
    println!("cargo:rerun-if-changed=../../schema/linear.graphql");
    cynic_codegen::register_schema("linear")
        .from_sdl_file("../../schema/linear.graphql")
        .unwrap()
        .as_default()
        .unwrap();
}
