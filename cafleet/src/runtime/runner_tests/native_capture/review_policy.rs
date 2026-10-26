use super::support::*;
use super::*;
use std::sync::Mutex;

#[derive(Default)]
struct Compiler {
    calls: Mutex<Vec<(String, usize)>>,
}
impl NativeCapturePatternCompiler for Compiler {
    fn compile(
        &self,
        pattern: &str,
        max_compiled_bytes: usize,
    ) -> Result<regex::Regex, crate::error::CafleetError> {
        self.calls
            .lock()
            .unwrap()
            .push((pattern.into(), max_compiled_bytes));
        BoundedCapturePatternCompiler.compile(pattern, max_compiled_bytes)
    }
}

#[test]
fn runner_collector_f2_whole_raw_policy_is_validated_before_the_first_compiler_invocation() {
    let limits = NativeCaptureLimits::default();
    for invalid in [
        "schema",
        "empty",
        "later_type",
        "count",
        "later_bytes",
        "aggregate",
        "sensitive",
        "later_sensitive_type",
    ] {
        let mut policy = redacted();
        policy["config"]["patterns"] = json!(["safe first pattern"]);
        match invalid {
            "schema" => policy["config"]["schema_version"] = json!(2),
            "empty" => policy["config"]["patterns"] = json!([]),
            "later_type" => policy["config"]["patterns"] = json!(["safe first pattern", 7]),
            "count" => {
                let mut patterns = vec!["safe".to_owned(); limits.max_patterns + 1];
                patterns[0] = "[".into();
                policy["config"]["patterns"] = json!(patterns);
            }
            "later_bytes" => {
                policy["config"]["patterns"] = json!([
                    "safe first pattern",
                    "x".repeat(limits.max_pattern_bytes + 1)
                ])
            }
            "aggregate" => {
                let width = limits.max_pattern_bytes / 2;
                let count = limits.max_total_pattern_bytes / width + 1;
                assert!(count <= limits.max_patterns);
                policy["config"]["patterns"] = json!(vec!["x".repeat(width); count]);
            }
            "sensitive" => policy["config"]["sensitive_rules"] = json!(["unknown-reviewed-rule"]),
            "later_sensitive_type" => {
                policy["config"]["sensitive_rules"] = json!(["synthetic-permission", 7])
            }
            _ => unreachable!(),
        }
        let fixture = CaptureFixture::new(policy);
        let compiler = Compiler::default();
        assert!(
            NativeCapturePolicy::from_launch_with_compiler(
                &fixture.spec,
                &[],
                &catalog(),
                &compiler
            )
            .is_err(),
            "{invalid}"
        );
        assert!(compiler.calls.lock().unwrap().is_empty(), "{invalid}");
    }
}

#[test]
fn runner_collector_f2_valid_patterns_compile_once_in_source_order_with_the_approved_cap() {
    let patterns = ["PRIVATE-[A-Z]+", "[0-9]+", "(?i)secret"];
    let mut policy = redacted();
    policy["config"]["patterns"] = json!(patterns);
    policy["config"]["sensitive_rules"] = json!(["synthetic-permission"]);
    let fixture = CaptureFixture::new(policy);
    let compiler = Compiler::default();
    assert!(
        NativeCapturePolicy::from_launch_with_compiler(&fixture.spec, &[], &catalog(), &compiler)
            .is_ok()
    );
    let expected: Vec<_> = patterns
        .into_iter()
        .map(|pattern| {
            (
                pattern.to_owned(),
                NativeCaptureLimits::default().max_compiled_regex_bytes,
            )
        })
        .collect();
    assert_eq!(*compiler.calls.lock().unwrap(), expected);
}

#[test]
fn runner_collector_f2_compact_pattern_expansion_stops_at_the_first_bounded_compilation() {
    let mut policy = redacted();
    policy["config"]["patterns"] = json!(["[a-z]{100000}"]);
    let fixture = CaptureFixture::new(policy);
    let compiler = Compiler::default();
    assert!(
        NativeCapturePolicy::from_launch_with_compiler(&fixture.spec, &[], &catalog(), &compiler)
            .is_err()
    );
    assert_eq!(
        *compiler.calls.lock().unwrap(),
        vec![(
            "[a-z]{100000}".into(),
            NativeCaptureLimits::default().max_compiled_regex_bytes
        )]
    );
}

#[test]
fn runner_collector_f2_metadata_policy_uses_no_regex_compiler() {
    let fixture = CaptureFixture::new(json!({"mode":"metadata"}));
    let compiler = Compiler::default();
    assert!(
        NativeCapturePolicy::from_launch_with_compiler(&fixture.spec, &[], &catalog(), &compiler)
            .is_ok()
    );
    assert!(compiler.calls.lock().unwrap().is_empty());
}

#[test]
fn runner_collector_f2_channel_selected_compiled_cap_remains_bounded_after_valid_construction() {
    let mut policy = redacted();
    policy["config"]["patterns"] = json!(["[a-z]{100}"]);
    let fixture = CaptureFixture::new(policy);
    let compiler = Compiler::default();
    let policy =
        NativeCapturePolicy::from_launch_with_compiler(&fixture.spec, &[], &catalog(), &compiler)
            .unwrap();
    let mut limits = NativeCaptureLimits::default();
    limits.max_compiled_regex_bytes = 128;
    assert!(native_capture_channel(&fixture.fixture.run, policy, limits).is_err());
    assert_eq!(compiler.calls.lock().unwrap().len(), 1);
}
