//! Hand-translated Widevine protobuf message types using prost derive macros.
//!
//! These types mirror the Widevine proto definitions used by the Widevine CDM
//! protocol. Because `protoc` is not required at build time, the messages are
//! defined directly in Rust using prost attributes.
//!
//! **ClientIdentification note**: the `.wvd` `client_id_blob` is the already-
//! serialised protobuf bytes of a full `ClientIdentification` message.  To
//! avoid deserialising and re-serialising that blob (which would require every
//! nested field), `LicenseRequest` stores it as a raw `bytes` field at field
//! number 1 — the same field number as `client_id` in the wire format.  Prost
//! encodes it verbatim, so the wire bytes are identical to what a full
//! `ClientIdentification` serialisation would produce.

// prost::Message derives Default, so we must NOT also derive Default manually.

// ── SignedMessage ─────────────────────────────────────────────────────────────

/// Wraps a DRM request or response along with its signature.
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignedMessage {
    /// `MessageType` discriminant.
    #[prost(int32, optional, tag = "1")]
    pub r#type: Option<i32>,
    /// Serialised inner message bytes.
    #[prost(bytes = "vec", optional, tag = "2")]
    pub msg: Option<Vec<u8>>,
    /// RSA-PSS-SHA1 signature over `msg`.
    #[prost(bytes = "vec", optional, tag = "3")]
    pub signature: Option<Vec<u8>>,
    /// RSA-OAEP-SHA1 encrypted session key.
    #[prost(bytes = "vec", optional, tag = "4")]
    pub session_key: Option<Vec<u8>>,
}

/// Discriminants for `SignedMessage.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum MessageType {
    LicenseRequest = 1,
    License = 2,
    ServiceCertificate = 5,
}

// ── LicenseRequest ────────────────────────────────────────────────────────────

/// A Widevine license request message.
///
/// Field 1 (`client_id_blob`) carries the raw serialised `ClientIdentification`
/// bytes from the `.wvd` file at the same wire tag as `client_id` in the proto
/// definition, so the encoded bytes are wire-compatible.
#[derive(Clone, PartialEq, prost::Message)]
pub struct LicenseRequest {
    /// Raw serialised `ClientIdentification` proto bytes (from `.wvd` file).
    /// Wire tag 1 matches `client_id` in the Widevine proto.
    #[prost(bytes = "vec", optional, tag = "1")]
    pub client_id_blob: Option<Vec<u8>>,

    /// Encrypted client identity (used in privacy / service-certificate mode).
    #[prost(message, optional, tag = "2")]
    pub encrypted_client_id: Option<EncryptedClientIdentification>,

    /// Content identification (PSSH-based).
    #[prost(message, optional, tag = "3")]
    pub content_id: Option<ContentIdentification>,

    /// `RequestType` discriminant.
    #[prost(int32, optional, tag = "4")]
    pub r#type: Option<i32>,

    /// Unix timestamp (seconds).
    #[prost(int64, optional, tag = "5")]
    pub request_time: Option<i64>,

    /// `ProtocolVersion` discriminant.
    #[prost(int32, optional, tag = "7")]
    pub protocol_version: Option<i32>,

    /// Random nonce for replay protection.
    #[prost(uint32, optional, tag = "9")]
    pub key_control_nonce: Option<u32>,
}

/// Discriminants for `LicenseRequest.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum RequestType {
    New = 1,
    Renewal = 2,
    Release = 3,
}

/// Content identification for a streaming/offline request.
#[derive(Clone, PartialEq, prost::Message)]
pub struct ContentIdentification {
    /// CENC (Common Encryption) content identification.
    #[prost(message, optional, tag = "1")]
    pub cenc_id_deprecated: Option<CencDeprecated>,
}

/// CENC-based content identification.
#[derive(Clone, PartialEq, prost::Message)]
pub struct CencDeprecated {
    /// One or more PSSH data blobs.
    #[prost(bytes = "vec", repeated, tag = "1")]
    pub pssh: Vec<Vec<u8>>,

    /// `LicenseType` discriminant.
    #[prost(int32, optional, tag = "2")]
    pub license_type: Option<i32>,

    /// Opaque request identifier.
    #[prost(bytes = "vec", optional, tag = "3")]
    pub request_id: Option<Vec<u8>>,
}

/// Discriminants for `CencDeprecated.license_type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum LicenseType {
    Streaming = 1,
    Offline = 2,
}

/// Encrypted client identity for privacy mode.
#[derive(Clone, PartialEq, prost::Message)]
pub struct EncryptedClientIdentification {
    #[prost(string, optional, tag = "1")]
    pub provider_id: Option<String>,

    #[prost(bytes = "vec", optional, tag = "2")]
    pub service_certificate_serial_number: Option<Vec<u8>>,

    #[prost(bytes = "vec", optional, tag = "3")]
    pub encrypted_client_id: Option<Vec<u8>>,

    #[prost(bytes = "vec", optional, tag = "4")]
    pub encrypted_client_id_iv: Option<Vec<u8>>,

    #[prost(bytes = "vec", optional, tag = "5")]
    pub encrypted_privacy_key: Option<Vec<u8>>,
}

// ── ProtocolVersion ───────────────────────────────────────────────────────────

/// Widevine protocol version discriminants.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum ProtocolVersion {
    Version21 = 21,
}

// ── License ───────────────────────────────────────────────────────────────────

/// A Widevine license containing one or more encrypted content keys.
#[derive(Clone, PartialEq, prost::Message)]
pub struct License {
    /// Encrypted content key containers.
    #[prost(message, repeated, tag = "3")]
    pub key: Vec<KeyContainer>,
}

/// A single encrypted content key within a `License`.
#[derive(Clone, PartialEq, prost::Message)]
pub struct KeyContainer {
    /// Key identifier bytes.
    #[prost(bytes = "vec", optional, tag = "1")]
    pub id: Option<Vec<u8>>,

    /// AES-128-CBC-PKCS7 IV for this key.
    #[prost(bytes = "vec", optional, tag = "2")]
    pub iv: Option<Vec<u8>>,

    /// Encrypted key bytes.
    #[prost(bytes = "vec", optional, tag = "3")]
    pub key: Option<Vec<u8>>,

    /// Key type discriminant (`KeyType`).
    #[prost(int32, optional, tag = "4")]
    pub r#type: Option<i32>,

    /// Track label (SD/HD/UHD1/UHD2/AUDIO).
    #[prost(string, optional, tag = "6")]
    pub track_label: Option<String>,
}

/// Key type discriminants for `KeyContainer.type`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum KeyType {
    Signing = 1,
    Content = 2,
    KeyControl = 3,
    OperatorSession = 4,
    Entitlement = 5,
    OemContent = 6,
}

// ── Service certificate types ─────────────────────────────────────────────────

/// A signed DRM device certificate (outer wrapper).
#[derive(Clone, PartialEq, prost::Message)]
pub struct SignedDrmDeviceCertificate {
    /// Serialised `DrmDeviceCertificate` bytes.
    #[prost(bytes = "vec", optional, tag = "1")]
    pub drm_certificate: Option<Vec<u8>>,

    /// RSA-PSS-SHA1 signature over `drm_certificate`.
    #[prost(bytes = "vec", optional, tag = "2")]
    pub signature: Option<Vec<u8>>,
}

/// DRM device certificate payload.
#[derive(Clone, PartialEq, prost::Message)]
pub struct DrmDeviceCertificate {
    /// Certificate type discriminant.
    #[prost(int32, optional, tag = "1")]
    pub r#type: Option<i32>,

    /// Serial number.
    #[prost(bytes = "vec", optional, tag = "2")]
    pub serial_number: Option<Vec<u8>>,

    /// Creation time (Unix seconds).
    #[prost(int64, optional, tag = "3")]
    pub creation_time_seconds: Option<i64>,

    /// DER-encoded RSA public key.
    #[prost(bytes = "vec", optional, tag = "4")]
    pub public_key: Option<Vec<u8>>,

    /// System ID.
    #[prost(uint32, optional, tag = "5")]
    pub system_id: Option<u32>,

    /// Test key indicator.
    #[prost(bool, optional, tag = "6")]
    pub test_device_deprecated: Option<bool>,

    /// Provider/issuer identifier.
    #[prost(string, optional, tag = "7")]
    pub provider_id: Option<String>,
}

/// Certificate type discriminants for `DrmDeviceCertificate`.
#[derive(Clone, Copy, Debug, PartialEq, Eq)]
#[repr(i32)]
pub enum CertificateType {
    Root = 0,
    Intermediate = 1,
    User = 2,
    Service = 3,
}
