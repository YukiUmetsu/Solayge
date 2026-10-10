//! Loopback integration tests for the QR pairing protocol.
//!
//! These drive the desktop [`PairingServer`] and phone [`PairingClient`] through
//! their public API exactly as a transport would, and assert the acceptance
//! cases: expired, replayed, canceled, and incorrectly-verified pairings are
//! rejected, and a revoked device fails on every reconnect.

use solayge_identity::{
    DeviceIdentity, IdentityError, PairingClient, PairingConfig, PairingOffer, PairingServer,
    PairingState, Scope,
};

const T0: i64 = 10_000;

fn server() -> PairingServer {
    PairingServer::new(DeviceIdentity::generate(), PairingConfig::default())
}

/// Drive begin + prove, returning the bound client and the offer.
fn begin_and_prove(server: &PairingServer, now: i64) -> (PairingClient, PairingOffer) {
    let offer = server.start(now).unwrap();
    let mut client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
    let response = server
        .handle_begin(client.begin_request(&offer).unwrap(), now + 1)
        .unwrap();
    client.accept_begin(&response).unwrap();
    let proof = client.prove_identity_request(&response).unwrap();
    server.handle_prove_identity(proof, now + 2).unwrap();
    (client, offer)
}

#[test]
fn a_full_pairing_registers_a_view_message_device() {
    let server = server();
    let (_client, offer) = begin_and_prove(&server, T0);
    let device = server.approve(&offer.pairing_id, None, T0 + 3).unwrap();

    assert_eq!(device.scopes, Scope::default_phone());
    assert!(!device.scopes.iter().any(|scope| scope.is_admin()));
    assert!(server.registry().is_authorized(&device.device_id));
    assert_eq!(
        server.session_state(&offer.pairing_id),
        Some(PairingState::Approved)
    );
}

#[test]
fn an_expired_pairing_is_rejected() {
    let server = server();
    let offer = server.start(T0).unwrap();
    let client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
    let err = server
        .handle_begin(client.begin_request(&offer).unwrap(), offer.expires_at + 1)
        .unwrap_err();
    assert!(matches!(err, IdentityError::PairingExpired(_)));
    assert!(server.registry().list().is_empty());
}

#[test]
fn a_replayed_pairing_is_single_use() {
    let server = server();
    let (client, offer) = begin_and_prove(&server, T0);
    server.approve(&offer.pairing_id, None, T0 + 3).unwrap();

    // Replaying the original challenge and approval is refused.
    assert!(matches!(
        server
            .handle_begin(client.begin_request(&offer).unwrap(), T0 + 4)
            .unwrap_err(),
        IdentityError::PairingNotActive(_)
    ));
    assert!(matches!(
        server.approve(&offer.pairing_id, None, T0 + 5).unwrap_err(),
        IdentityError::PairingNotActive(_)
    ));
}

#[test]
fn a_canceled_pairing_is_rejected() {
    let server = server();
    let (_client, offer) = begin_and_prove(&server, T0);
    server.reject(&offer.pairing_id, T0 + 3).unwrap();
    assert!(server.registry().list().is_empty());
    assert!(matches!(
        server.approve(&offer.pairing_id, None, T0 + 4).unwrap_err(),
        IdentityError::PairingRejected(_)
    ));
}

#[test]
fn an_incorrectly_verified_pairing_is_rejected_and_canceled() {
    let server = server();
    let offer = server.start(T0).unwrap();
    let mut client = PairingClient::new(DeviceIdentity::generate(), "iPhone", "ios");
    let response = server
        .handle_begin(client.begin_request(&offer).unwrap(), T0 + 1)
        .unwrap();
    client.accept_begin(&response).unwrap();

    let attacker = DeviceIdentity::generate();
    let mut forged = client.prove_identity_request(&response).unwrap();
    forged.device_proof = attacker.sign_base64(b"not the transcript");

    for attempt in 0..3 {
        assert!(matches!(
            server
                .handle_prove_identity(forged.clone(), T0 + 2 + attempt)
                .unwrap_err(),
            IdentityError::ProofInvalid(_)
        ));
    }
    assert_eq!(
        server.session_state(&offer.pairing_id),
        Some(PairingState::Rejected)
    );
    assert!(server.registry().list().is_empty());
}

#[test]
fn a_revoked_device_fails_on_every_reconnect() {
    let server = server();
    let (_client, offer) = begin_and_prove(&server, T0);
    let device = server.approve(&offer.pairing_id, None, T0 + 3).unwrap();
    let id = device.device_id.clone();
    assert!(server.registry().authorize(&id, T0 + 4).is_ok());

    server.registry().revoke(&id);

    for attempt in 0..5 {
        assert!(!server.registry().is_authorized(&id));
        assert!(matches!(
            server
                .registry()
                .authorize(&id, T0 + 10 + attempt)
                .unwrap_err(),
            IdentityError::DeviceRevoked(_)
        ));
    }

    // A fresh pairing authorizes a new device, while the revoked id stays
    // blocked until that exact identity is approved again.
    let (_client2, offer2) = begin_and_prove(&server, T0 + 20);
    let repaired = server.approve(&offer2.pairing_id, None, T0 + 23).unwrap();
    assert_ne!(repaired.device_id, id);
    assert!(server.registry().is_authorized(&repaired.device_id));
    assert!(!server.registry().is_authorized(&id));
}
