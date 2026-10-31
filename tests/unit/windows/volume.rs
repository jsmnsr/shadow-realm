use super::*;

#[test]
fn attribute_request_is_scoped_to_one_volume_without_revert_on_close() {
    let input = attribute_request(0xe000_0000_0000_0000);
    assert_eq!(input.GptAttributes, 0xe000_0000_0000_0000);
    assert!(!input.RevertOnClose);
    assert!(!input.ApplyToAllConnectedVolumes);
    assert_eq!(input.Reserved1, 0);
    assert_eq!(input.Reserved2, 0);
}
