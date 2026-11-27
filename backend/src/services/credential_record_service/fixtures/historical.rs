//! Unmarked v2 ciphertext from the 4f828cc49 server tests, using
//! `test_encryption_keys()`. Keep the source bytes independent of the writer
//! being exercised by migration, exact-copy and first-marker tests.

pub(crate) fn empty() -> Vec<u8> {
    hex::decode("0202003c0ccd52892abc850344c0fb2419596ff950f9ffad9f12320dc2a1874ad6d63a581224eb122797bfe782589299a84919dd2b203d37d0a73be094a6b28afa316224a8753a1373b50fb931a1c0e33bfd0043a0d6c1c71c5ed9af").unwrap()
}

pub(crate) fn shared_access() -> Vec<u8> {
    hex::decode("0202003cf7f0ff2f8d894bc2de314a5f83d1780cc5377abc6fd3c8d3c98fb04e665c8099ac36002e19f2a054428408fe033854faa252ce00a380d1d28caee3435e5009e2d16db623a581b4bdeeca1b4e85f31b2e10355bf3c2c817c12000cf8fb930bdb27db06eb083d263714fed96").unwrap()
}

pub(crate) fn first_marker_token() -> Vec<u8> {
    hex::decode("0202003c84eda7ad93cd64523d10b79f21a8f55a630d816839f24c7fc7911e235d59c2229ca72bdf392319e60776c958027c26866182e2ed270c756f33c297eead3c35be9df4503716c916717f3788b399575bca1d1db649a1cfc8cb207083cfa02f2286df61e85bbd02c964b271").unwrap()
}

/// Each legacy integer array contained the UTF-8 value `distinctive-material-{field}`.
pub(crate) fn credential_field(field: &str) -> Vec<u8> {
    let ciphertext = match field {
        "access_token_encrypted" => {
            "0202003c4fcd8ae6b805de5cbf630e7f30537bd69f304c09914e245d2c2fb73a8537a43b6e934af0607d400e96beb3409756d9d365327e6302e6b6f2f897ac5d4801c3a69b29bdcc73210086b0a31781bded92367fc74019782bd1372345e47a443a2b76f01c3adb8e458a0edd2b7e9798c70864e15965ec57eaaff206aefdb4f2f553f95c6c23"
        }
        "api_key_encrypted" => {
            "0202003cc0e03b6cb436c2dd264844b11d9ee31591ce351db5025cb250d6c86efc2ab5e995af3893c5cb5d29390250b4a2315584984c5b1a3fac011118cf4af16a66ad82c09d864eec8a38c62dcc8b76882eb9c1eaaa790d2b86af1aa7eb3baca62520530126473f150e7f8998dba2176c07ad8c3c8414ac1de4fb88ee8120d262b3"
        }
        "client_id_encrypted" => {
            "0202003c4596ee247fbe566d734474bae260842e6ba202f3946ea33623f64d8b6e619b3064644703595ffb70cb63ce96b3e075cddd7308f56e35291275dab26933d06d48d565a2ab1d78bdf02e4f38f85a5588c252ee01897c890437ed88d7cb0c90d4a85b1c38d87800612a9eb90b135f65f7f3a71d5bc9e4ec197278809c68d594e2cf"
        }
        "client_secret_encrypted" => {
            "0202003cde52675b6facd7eb0f07b71353ba6122334e7292da56ea1028c4e53e126f7bc2e4dc834756f15a3e1c2eb9e7df0b370a84550d094f8629f05f6fb517928a876b3925fb830ff5263fafa1e7444a2c596a6ea0bc8c89e8931c375b2d63aae8e88c082672abd88694eaa8dadda37c1ad6ddf597dd7f9f674c1ecf3b99d9c98d341076cf76ad"
        }
        "credential_encrypted" => {
            "0202003c0b9aee5b28451f31140dddc87b45d3ff95efb818154d7a47bfeb86a7b444d1b506fa4f70c91b3d93737096f64a09733809538a7734fe20fd9fa290fe11a98b59ff52a67fe4195be140900ffc8ff8f85f015735ca6b7d04deac3cccd517a0d215caab056ea090849997988b102070cff8a05c167aa9cc7923840ac77f598eddb95d"
        }
        "refresh_token_encrypted" => {
            "0202003c885c66a9428ab4e1c15620db1a4f8d49214eeb95c285440a982a613f48c70f8cd2c372675254dcc75512614dc02a936750d879230f758b9839e6fb8c2638acacf92a15cde3385fdb86da5edf83da7e90bf817501a5299cbf38a9cbd752fc9e99e03272e84f6c3f87b4cedc4479b2174ce1d7296635354a9a35916d48b70fdff62ecec638"
        }
        "user_oauth_client_id_encrypted" => {
            "0202003ca5e2d697f3d86acdcee283bdc82f660401323a94db8f6e90ce2722982f1c6f975eb04aa19fc17f2c19060ee548d2d863228dd623f76e2a78255da340d037c16ae12b5db0f2227bed58784a7450275a3c3c6c1e4bbeb4f433738d7b8b3738e124aab47b475145cb12d975bddfb4b89000f2bf77c3bfe7d1006b1e49ef09ec8d272263aa97da576ee015dcf9"
        }
        "user_oauth_client_secret_encrypted" => {
            "0202003cf75953e0e3b7ed30834909262a75bb768bd0868ffb59429126a863ffd414900556959978884ca048c802a39c8f010ceabc86977408928cadd24a32cac08940b3bb9ec3dab1b980859f50894b825cc43ccc737f89a2477a795601f892e0a00f334cee7b3bcba12b71428750d04861be1156aa322b7d9914568f4a4da34f3e496d02792b2b0b48df4a5600d8a68e4230"
        }
        _ => panic!("missing historical credential fixture for {field}"),
    };
    hex::decode(ciphertext).unwrap()
}
