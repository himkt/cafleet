use super::*;

const PAYLOAD_CANARY: &str = "WIRE_R1_PRIVATE_LITERAL_CANARY";
const IDENTITY_CANARY: &str = "WIRE_R1_PRIVATE_IDENTITY_CANARY";

fn control(case: &Case) -> Value {
    let mut good = case.request("submit_prompt", None);
    good["literal"] = json!(PAYLOAD_CANARY);
    let request = decoded(&good);
    assert_eq!(
        request.identity,
        ControlIdentity::from_run(&case.fixture.run)
    );
    assert_frame(&encode_input_request(&request).unwrap(), &good);
    good
}

fn private_error(error: &CafleetError, good: &Value) {
    let diagnostic = format!("{error:?}\n{error}\n{}", error.message());
    for private in [
        PAYLOAD_CANARY,
        IDENTITY_CANARY,
        good["identity"]["database_identity_json"]["canonical_path"]
            .as_str()
            .unwrap(),
        good["identity"]["database_identity_json"]["instance_id"]
            .as_str()
            .unwrap(),
        good["identity"]["launch_nonce"].as_str().unwrap(),
    ] {
        assert!(
            !diagnostic.contains(private),
            "identity validation preserves payload privacy"
        );
    }
}

fn reject_decode(good: &Value, bad: &Value, label: &str) {
    match decode_input_request(&bytes(bad)) {
        Err(error) => private_error(&error, good),
        Ok(_) => panic!("owning identity schema rejects decode case: {label}"),
    }
}

fn reject_encode(good: &Value, database: Value, nonce: &str, label: &str) {
    let mut request = decoded(good);
    request.identity.database_identity_json = database;
    request.identity.launch_nonce = nonce.into();
    match encode_input_request(&request) {
        Err(error) => private_error(&error, good),
        Ok(_) => panic!("owning identity schema rejects constructed encode case: {label}"),
    }
}

fn bad_paths() -> Vec<Value> {
    vec![
        json!(format!("{IDENTITY_CANARY}/relative.sqlite")),
        json!(""),
        json!("."),
        json!("../database.sqlite"),
        Value::Null,
        json!(17),
        json!(true),
        json!([]),
        json!({}),
    ]
}

fn bad_hex(length: usize) -> Vec<Value> {
    vec![
        json!(""),
        json!("a".repeat(length - 1)),
        json!("a".repeat(length + 1)),
        json!("g".repeat(length)),
        json!(format!("{IDENTITY_CANARY}{}", "g".repeat(length))),
        json!(format!("{} ", "a".repeat(length - 1))),
        json!("é".repeat(length / 2)),
        Value::Null,
        json!(17),
        json!(true),
        json!([]),
        json!({}),
    ]
}

fn bad_numbers() -> Vec<Value> {
    vec![
        json!(-1),
        json!(i64::MIN),
        json!(1.0),
        json!(1.5),
        json!(18446744073709551616.0_f64),
        Value::Null,
        json!(IDENTITY_CANARY),
        json!(true),
        json!([]),
        json!({}),
    ]
}

#[test]
fn step5_wire_r1_genuine_and_owning_domain_controls_preserve_literal_identity() {
    let case = Case::new();
    let good = control(&case);
    let mut uppercase = good.clone();
    uppercase["identity"]["database_identity_json"]["instance_id"] =
        json!("ABCDEF0123456789".repeat(2));
    uppercase["identity"]["launch_nonce"] = json!("ABCDEF0123456789".repeat(4));
    let mut mixed = uppercase.clone();
    mixed["identity"]["database_identity_json"]["instance_id"] =
        json!("aBcDeF0123456789".repeat(2));
    mixed["identity"]["launch_nonce"] = json!("aBcDeF0123456789".repeat(4));
    let mut extended = good.clone();
    extended["identity"]["database_identity_json"]["extension"] =
        json!({"fixture":IDENTITY_CANARY,"nested":[1,null]});
    let mut bounds = good.clone();
    bounds["identity"]["database_identity_json"]["device"] = json!(0);
    bounds["identity"]["database_identity_json"]["inode"] = json!(u64::MAX);
    let mut reverse_bounds = good.clone();
    reverse_bounds["identity"]["database_identity_json"]["device"] = json!(u64::MAX);
    reverse_bounds["identity"]["database_identity_json"]["inode"] = json!(0);
    let mut lexical_absolute = good.clone();
    lexical_absolute["identity"]["database_identity_json"]["canonical_path"] = json!(
        case.fixture
            ._dir
            .path()
            .join("absent-parent/../unallocated.sqlite")
    );
    for value in [
        good,
        uppercase,
        mixed,
        extended,
        bounds,
        reverse_bounds,
        lexical_absolute,
    ] {
        let decoded = decode_input_request(&bytes(&value)).unwrap();
        assert_eq!(
            decoded.identity.database_identity_json,
            value["identity"]["database_identity_json"]
        );
        assert_eq!(
            decoded.identity.launch_nonce,
            value["identity"]["launch_nonce"].as_str().unwrap()
        );
        assert_frame(&encode_input_request(&decoded).unwrap(), &value);
    }
}

#[test]
fn step5_wire_r1_decode_requires_each_owning_database_identity_field() {
    let case = Case::new();
    let good = control(&case);
    for field in ["canonical_path", "instance_id", "device", "inode"] {
        let mut bad = good.clone();
        bad["identity"]["database_identity_json"]
            .as_object_mut()
            .unwrap()
            .remove(field)
            .unwrap();
        reject_decode(&good, &bad, field);
    }
}

#[test]
fn step5_wire_r1_decode_requires_an_absolute_string_database_path() {
    let case = Case::new();
    let good = control(&case);
    for (index, value) in bad_paths().into_iter().enumerate() {
        let mut bad = good.clone();
        bad["identity"]["database_identity_json"]["canonical_path"] = value;
        reject_decode(&good, &bad, &format!("canonical_path {index}"));
    }
}

#[test]
fn step5_wire_r1_decode_requires_a_32_character_ascii_hex_instance_id() {
    let case = Case::new();
    let good = control(&case);
    for (index, value) in bad_hex(32).into_iter().enumerate() {
        let mut bad = good.clone();
        bad["identity"]["database_identity_json"]["instance_id"] = value;
        reject_decode(&good, &bad, &format!("instance_id {index}"));
    }
}

fn reject_numeric_field(field: &str) {
    let case = Case::new();
    let good = control(&case);
    for (index, value) in bad_numbers().into_iter().enumerate() {
        let mut bad = good.clone();
        bad["identity"]["database_identity_json"][field] = value;
        reject_decode(&good, &bad, &format!("{field} {index}"));
    }
}

#[test]
fn step5_wire_r1_decode_requires_an_unsigned_64_bit_device() {
    reject_numeric_field("device");
}

#[test]
fn step5_wire_r1_decode_requires_an_unsigned_64_bit_inode() {
    reject_numeric_field("inode");
}

#[test]
fn step5_wire_r1_decode_requires_a_64_character_ascii_hex_launch_nonce() {
    let case = Case::new();
    let good = control(&case);
    for (index, value) in bad_hex(64).into_iter().enumerate() {
        let mut bad = good.clone();
        bad["identity"]["launch_nonce"] = value;
        reject_decode(&good, &bad, &format!("launch_nonce {index}"));
    }
    let mut missing = good.clone();
    missing["identity"]
        .as_object_mut()
        .unwrap()
        .remove("launch_nonce")
        .unwrap();
    reject_decode(&good, &missing, "missing launch_nonce");
}

#[test]
fn step5_wire_r1_encoder_requires_each_owning_database_identity_field() {
    let case = Case::new();
    let good = control(&case);
    for field in ["canonical_path", "instance_id", "device", "inode"] {
        let mut database = good["identity"]["database_identity_json"].clone();
        database.as_object_mut().unwrap().remove(field).unwrap();
        reject_encode(&good, database, &case.fixture.run.launch_nonce, field);
    }
}

#[test]
fn step5_wire_r1_encoder_validates_constructed_database_path_and_instance_id() {
    let case = Case::new();
    let good = control(&case);
    for (field, values) in [
        ("canonical_path", bad_paths()),
        ("instance_id", bad_hex(32)),
    ] {
        for (index, value) in values.into_iter().enumerate() {
            let mut database = good["identity"]["database_identity_json"].clone();
            database[field] = value;
            reject_encode(
                &good,
                database,
                &case.fixture.run.launch_nonce,
                &format!("{field} {index}"),
            );
        }
    }
}

#[test]
fn step5_wire_r1_encoder_validates_constructed_unsigned_device_and_inode() {
    let case = Case::new();
    let good = control(&case);
    for field in ["device", "inode"] {
        for (index, value) in bad_numbers().into_iter().enumerate() {
            let mut database = good["identity"]["database_identity_json"].clone();
            database[field] = value;
            reject_encode(
                &good,
                database,
                &case.fixture.run.launch_nonce,
                &format!("{field} {index}"),
            );
        }
    }
}

#[test]
fn step5_wire_r1_encoder_validates_a_constructed_launch_nonce() {
    let case = Case::new();
    let good = control(&case);
    for value in bad_hex(64) {
        if let Some(nonce) = value.as_str() {
            reject_encode(
                &good,
                good["identity"]["database_identity_json"].clone(),
                nonce,
                "launch_nonce",
            );
        }
    }
}
