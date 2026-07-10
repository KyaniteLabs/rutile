#[test]
fn production_runner_provider_and_test_trust_are_not_external_api() {
    let cases = trybuild::TestCases::new();
    cases.compile_fail("tests/ui/runner_provider_private.rs");
    cases.compile_fail("tests/ui/runner_test_trust_absent.rs");
    cases.compile_fail("tests/ui/runner_capabilities_private.rs");
}
