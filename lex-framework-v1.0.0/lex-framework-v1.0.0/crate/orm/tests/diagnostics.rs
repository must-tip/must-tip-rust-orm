#[test]
fn invalid_models_explain_the_declaration_error() {
    let tests = trybuild::TestCases::new();
    tests.compile_fail("tests/ui/*.rs");
}
