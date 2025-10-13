//! Basic integration test to verify test infrastructure works.

mod common;

use common::ReservationFixture;

/// Test that the fixture builder works correctly.
#[test]
fn test_fixture_basic() {
    let reservation = ReservationFixture::new().build();
    assert_eq!(reservation.port().value(), 8080);
}

/// Test that fixtures can be customized.
#[test]
fn test_fixture_custom() {
    let reservation = ReservationFixture::new()
        .with_port(9000)
        .with_project("test-project")
        .build();

    assert_eq!(reservation.port().value(), 9000);
    assert_eq!(reservation.project(), Some("test-project"));
}
