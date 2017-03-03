use std::collections::HashMap;
use std::io::Read;
use std::result;

use reqwest::{self, header, Client, Method, Response};
use reqwest::header::Headers;
// use hyper::{self, header, Client};
// use hyper::client::response::Response;
use rustc_serialize::{json, Decodable, Decoder, Encodable, Encoder};

use client::error::{Error, Result};

use std::time::Duration;
use chrono::{DateTime, FixedOffset, NaiveDateTime};
use url::Url;
use TryInto;

/// Errors
pub mod error;

/// Lease duration.
///
/// Note: Value returned from vault api is assumed to be in seconds.
///
/// ```
/// use hashicorp_vault::client::VaultDuration;
///
/// assert_eq!(VaultDuration::days(1),
///            VaultDuration(std::time::Duration::from_secs(86400)));
/// ```
#[derive(Debug, PartialEq, Eq, PartialOrd, Ord)]
pub struct VaultDuration(pub Duration);

impl VaultDuration {
    /// Construct a duration from some number of seconds.
    pub fn seconds(s: u64) -> VaultDuration {
        VaultDuration(Duration::from_secs(s))
    }

    /// Construct a duration from some number of minutes.
    pub fn minutes(m: u64) -> VaultDuration {
        VaultDuration::seconds(m * 60)
    }

    /// Construct a duration from some number of hours.
    pub fn hours(h: u64) -> VaultDuration {
        VaultDuration::minutes(h * 60)
    }

    /// Construct a duration from some number of days.
    pub fn days(d: u64) -> VaultDuration {
        VaultDuration::hours(d * 24)
    }
}


impl Decodable for VaultDuration {
    fn decode<D: Decoder>(d: &mut D) -> ::std::result::Result<VaultDuration, D::Error> {
        let num = try!(d.read_u64());
        Ok(VaultDuration(Duration::from_secs(num)))
    }
}

impl Encodable for VaultDuration {
    fn encode<S: Encoder>(&self, s: &mut S) -> result::Result<(), S::Error> {
        s.emit_u64(self.0.as_secs())
    }
}

/// Used for vault responses that return seconds since unix epoch
/// See: https://github.com/hashicorp/vault/issues/1654
#[derive(Debug)]
pub struct VaultNaiveDateTime(pub NaiveDateTime);
impl Decodable for VaultNaiveDateTime {
    fn decode<D: Decoder>(d: &mut D) -> ::std::result::Result<VaultNaiveDateTime, D::Error> {
        let seconds_since_epoch = try!(d.read_i64());
        let date_time = NaiveDateTime::from_timestamp_opt(seconds_since_epoch, 0);

        match date_time {
            Some(dt) => Ok(VaultNaiveDateTime(dt)),
            None => {
                Err(d.error(&format!("Could not parse: `{}` as a unix timestamp",
                                     seconds_since_epoch,
                                     )))
            }
        }
    }
}

/// Used for responses that return RFC 3339 timestamps
/// See: https://github.com/hashicorp/vault/issues/1654
#[derive(Debug)]
pub struct VaultDateTime(pub DateTime<FixedOffset>);
impl Decodable for VaultDateTime {
    fn decode<D: Decoder>(d: &mut D) -> ::std::result::Result<VaultDateTime, D::Error> {
        let ts = try!(d.read_str());
        let date_time = DateTime::parse_from_rfc3339(&ts);

        match date_time {
            Ok(dt) => Ok(VaultDateTime(dt)),
            Err(e) => {
                Err(d.error(&format!("Could not parse: `{}` as an RFC 3339 timestamp. Error: \
                                      `{:?}`",
                                     ts,
                                     e)))
            }
        }
    }
}

/// Vault client used to make API requests to the vault
#[derive(Debug)]
pub struct VaultClient<T>
    where T: Decodable
{
    /// URL to vault instance
    pub host: Url,
    /// Token to access vault
    pub token: String,
    /// `hyper::Client`
    client: Client,
    /// Data
    pub data: Option<VaultResponse<T>>,
}

/// Token data, used in `VaultResponse`
#[derive(RustcDecodable, Debug)]
pub struct TokenData {
    /// Accessor token
    pub accessor: Option<String>,
    /// Creation time
    pub creation_time: VaultNaiveDateTime,
    /// Creation time-to-live
    pub creation_ttl: Option<VaultDuration>,
    /// Display name
    pub display_name: String,
    /// Max time-to-live
    pub explicit_max_ttl: Option<VaultDuration>,
    /// Token id
    pub id: String,
    /// Last renewal time
    pub last_renewal_time: Option<VaultDuration>,
    /// Meta
    pub meta: Option<HashMap<String, String>>,
    /// Number of uses (0: unlimited)
    pub num_uses: u64,
    /// true if token is an orphan
    pub orphan: bool,
    /// Path
    pub path: String,
    /// Policies for token
    pub policies: Vec<String>,
    /// True if renewable
    pub renewable: Option<bool>,
    /// Role
    pub role: Option<String>,
    /// Time-to-live
    pub ttl: VaultDuration,
}

/// Secret data, used in `VaultResponse`
#[derive(RustcDecodable, RustcEncodable, Debug)]
struct SecretData {
    value: String,
}

/// Vault auth
#[derive(RustcDecodable, Debug)]
pub struct Auth {
    /// Client token id
    pub client_token: String,
    /// Accessor
    pub accessor: Option<String>,
    /// Policies
    pub policies: Vec<String>,
    /// Metadata
    pub metadata: Option<HashMap<String, String>>,
    /// Lease duration
    pub lease_duration: Option<VaultDuration>,
    /// True if renewable
    pub renewable: bool,
}

/// Vault response. Different vault responses have different `data` types, so `D` is used to
/// represent this.
#[derive(RustcDecodable, Debug)]
pub struct VaultResponse<D>
    where D: Decodable
{
    /// Request id
    #[cfg(feature = "vault_0.6.1")]
    pub request_id: String,
    /// Lease id
    pub lease_id: Option<String>,
    /// True if renewable
    pub renewable: Option<bool>,
    /// Lease duration (in seconds)
    pub lease_duration: Option<VaultDuration>,
    /// Data
    pub data: Option<D>,
    /// Warnings
    pub warnings: Option<Vec<String>>,
    /// Auth
    pub auth: Option<Auth>,
    /// Wrap info, containing token to perform unwrapping
    pub wrap_info: Option<WrapInfo>,
}

/// Information provided to retrieve a wrapped response
#[derive(RustcDecodable, Debug)]
pub struct WrapInfo {
    /// Time-to-live
    pub ttl: VaultDuration,
    /// Token
    pub token: String,
    /// Creation time, note this returned in RFC 3339 format
    pub creation_time: VaultDateTime,
    /// Wrapped accessor
    pub wrapped_accessor: Option<String>,
}

/// Wrapped response is serialized json
#[derive(RustcDecodable, RustcEncodable, Debug)]
pub struct WrapData {
    /// Serialized json string of type `VaultResponse<HashMap<String, String>>`
    response: String,
}

/// `AppRole` properties
#[derive(RustcDecodable, Debug)]
pub struct AppRoleProperties {
    /// Require `secret_id` to be presented when logging in using this `AppRole`. Defaults to 'true'.
    pub bind_secret_id: bool,
    /// Comma-separated list of CIDR blocks; if set, specifies blocks of IP addresses which can
    /// perform the login operation.
    pub bound_cidr_list: String,
    /// If set, the token generated using this `AppRole` is a periodic token; so long as it is
    /// renewed it never expires, but the TTL set on the token at each renewal is fixed to the value
    /// specified here. If this value is modified, the token will pick up the new value at its next
    /// renewal.
    pub period: VaultDuration,
    /// List of policies set on tokens issued via this `AppRole`.
    pub policies: Vec<String>,
    /// Number of times any particular `SecretID` can be used to fetch a token from this `AppRole`,
    /// after which the `SecretID` will expire.
    pub secret_id_num_uses: u64,
    /// Duration after which any `SecretID` expires.
    pub secret_id_ttl: VaultDuration,
    /// Duration after which the issued token can no longer be renewed.
    pub token_max_ttl: VaultDuration,
}

/// Payload to send to vault when authenticating via `AppId`
#[derive(RustcDecodable, RustcEncodable, Debug)]
struct AppIdPayload {
    app_id: String,
    user_id: String,
}

/// Payload to send to vault when authenticating via `AppRole`
#[derive(RustcDecodable, RustcEncodable, Debug)]
struct AppRolePayload {
    role_id: String,
    secret_id: Option<String>,
}

/// Postgresql secret backend
#[derive(RustcDecodable, RustcEncodable, Debug)]
pub struct PostgresqlLogin {
    /// Password
    pub password: String,
    /// Username
    pub username: String,
}

/// Response sent by vault when listing policies.  We hide this from the
/// caller.
#[derive(RustcDecodable, RustcEncodable, Debug)]
struct PoliciesResponse {
    policies: Vec<String>,
}

/// Response sent by vault when issuing a `LIST` request.
#[derive(RustcDecodable, RustcEncodable, Debug)]
pub struct ListResponse {
    /// keys will include the items listed
    pub keys: Vec<String>,
}

/// Options that we use when renewing leases on tokens and secrets.
#[derive(RustcDecodable, RustcEncodable, Debug)]
struct RenewOptions {
    /// The amount of time for which to renew the lease.  May be ignored or
    /// overriden by vault.
    increment: Option<u64>,
}

/// Options for creating a token.  This is intended to be used as a
/// "builder"-style interface, where you create a new `TokenOptions`
/// object, call a bunch of chained methods on it, and then pass the result
/// to `Client::create_token`.
///
/// ```
/// use hashicorp_vault::client::{TokenOptions, VaultDuration};
///
/// let _ = TokenOptions::default()
///   .id("test12345")
///   .policies(vec!("root"))
///   .default_policy(false)
///   .orphan(true)
///   .renewable(false)
///   .display_name("jdoe-temp")
///   .number_of_uses(10)
///   .ttl(VaultDuration::hours(3))
///   .explicit_max_ttl(VaultDuration::hours(13));
/// ```
///
/// If an option is not specified, it will be set according to [Vault's
/// standard defaults for newly-created tokens][token].
///
/// [token]: https://www.vaultproject.io/docs/auth/token.html
#[derive(Default, RustcEncodable, Debug)]
pub struct TokenOptions {
    id: Option<String>,
    policies: Option<Vec<String>>,
    // TODO: `meta`
    no_parent: Option<bool>,
    no_default_policy: Option<bool>,
    renewable: Option<bool>,
    ttl: Option<String>,
    explicit_max_ttl: Option<String>,
    display_name: Option<String>,
    num_uses: Option<u64>,
}

impl TokenOptions {
    /// Set the `id` of the created token to the specified value.  **This
    /// may make it easy for attackers to guess your token.** Typically,
    /// this is used for testing and similar purposes.
    pub fn id<S: Into<String>>(mut self, id: S) -> Self {
        self.id = Some(id.into());
        self
    }

    /// Supply a list of policies that will be used to grant permissions to
    /// the created token.  Unless you also call `default_policy(false)`, the
    /// policy `default` will be added to this list in modern versions of
    /// vault.
    pub fn policies<I>(mut self, policies: I) -> Self
        where I: IntoIterator,
              I::Item: Into<String>
    {
        self.policies = Some(policies.into_iter().map(|p| p.into()).collect());
        self
    }

    /// Should we grant access to the `default` policy?  Defaults to true.
    pub fn default_policy(mut self, enable: bool) -> Self {
        self.no_default_policy = Some(!enable);
        self
    }

    /// Should this token be an "orphan", allowing it to survive even when
    /// the token that created it expires or is revoked?
    pub fn orphan(mut self, orphan: bool) -> Self {
        self.no_parent = Some(!orphan);
        self
    }

    /// Should the token be renewable?
    pub fn renewable(mut self, renewable: bool) -> Self {
        self.renewable = Some(renewable);
        self
    }

    /// For various logging purposes, what should this token be called?
    pub fn display_name<S>(mut self, name: S) -> Self
        where S: Into<String>
    {
        self.display_name = Some(name.into());
        self
    }

    /// How many times can this token be used before it stops working?
    pub fn number_of_uses(mut self, uses: u64) -> Self {
        self.num_uses = Some(uses);
        self
    }

    /// How long should this token remain valid for?
    pub fn ttl<D: Into<VaultDuration>>(mut self, ttl: D) -> Self {
        self.ttl = Some(format!("{}s", ttl.into().0.as_secs()));
        self
    }

    /// How long should this token remain valid for, even if it is renewed
    /// repeatedly?
    pub fn explicit_max_ttl<D: Into<VaultDuration>>(mut self, ttl: D) -> Self {
        self.explicit_max_ttl = Some(format!("{}s", ttl.into().0.as_secs()));
        self
    }
}

/// http verbs
#[derive(Debug)]
pub enum HttpVerb {
    /// GET
    GET,
    /// POST
    POST,
    /// PUT
    PUT,
    /// DELETE
    DELETE,
    /// LIST
    LIST,
}

/// endpoint response variants
#[derive(Debug)]
pub enum EndpointResponse<D>
    where D: Decodable
{
    /// Vault response
    VaultResponse(VaultResponse<D>),
    /// Empty, but still successful response
    Empty,
}

header! {
    /// Token used to authenticate with the vault API
    (XVaultToken, "X-Vault-Token") => [String]
}
header! {
    /// The TTL for the token is set by the client using the `X-Vault-Wrap-TTL` header and can be
    /// either an integer number of seconds or a string duration of seconds (15s), minutes (20m),
    /// or hours (25h). When using the Vault CLI, you can set this via the -wrap-ttl parameter.
    /// Response wrapping is per-request; it is the presence of a value in this header that
    /// activates wrapping of the response.
    ///
    /// See: https://www.vaultproject.io/docs/secrets/cubbyhole/index.html
    (XVaultWrapTTL, "X-Vault-Wrap-TTL") => [String]
}

impl VaultClient<TokenData> {
    /// Construct a `VaultClient` from an existing vault token
    pub fn new<U>(host: U, token: &str) -> Result<VaultClient<TokenData>>
        where U: TryInto<Url, Err = Error>
    {
        let host = try!(host.try_into());
        let client = Client::new()?;
        let mut res = try!(
            handle_hyper_response(client.get(try!(host.join("/v1/auth/token/lookup-self")))
                                  .header(XVaultToken(token.to_string()))
                                  .send()));
        let decoded: VaultResponse<TokenData> = try!(parse_vault_response(&mut res));
        Ok(VaultClient {
            host: host,
            token: token.to_string(),
            client: client,
            data: Some(decoded),
        })
    }
}

impl VaultClient<()> {
    /// Construct a `VaultClient` via the `App ID`
    /// [auth backend](https://www.vaultproject.io/docs/auth/app-id.html)
    ///
    /// NOTE: This backend is now deprecated by vault.
    #[deprecated(since = "0.6.1")]
    pub fn new_app_id<U>(host: U, app_id: &str, user_id: &str) -> Result<VaultClient<()>>
        where U: TryInto<Url, Err = Error>
    {
        let host = try!(host.try_into());
        let client = Client::new()?;
        let payload = try!(json::encode(&AppIdPayload {
            app_id: app_id.to_string(),
            user_id: user_id.to_string(),
        }));
        let mut res =
            try!(handle_hyper_response(client.post(try!(host.join("/v1/auth/app-id/login")))
                .body(payload)
                .send()));
        let decoded: VaultResponse<()> = try!(parse_vault_response(&mut res));
        let token = match decoded.auth {
            Some(ref auth) => auth.client_token.clone(),
            None => {
                return Err(Error::Vault(format!("No client token found in response: `{:?}`",
                                                &decoded.auth)))
            }
        };
        Ok(VaultClient {
            host: host,
            token: token,
            client: client,
            data: Some(decoded),
        })
    }

    /// Construct a `VaultClient` via the `AppRole`
    /// [auth backend](https://www.vaultproject.io/docs/auth/approle.html)
    pub fn new_app_role<U, R, S>(host: U,
                                 role_id: R,
                                 secret_id: Option<S>)
                                 -> Result<VaultClient<()>>
        where U: TryInto<Url, Err = Error>,
              R: Into<String>,
              S: Into<String>
    {
        let host = try!(host.try_into());
        let client = Client::new()?;
        let secret_id = match secret_id {
            Some(s) => Some(s.into()),
            None => None,
        };
        let payload = try!(json::encode(&AppRolePayload {
            role_id: role_id.into(),
            secret_id: secret_id,
        }));
        let mut res =
            try!(handle_hyper_response(client.post(try!(host.join("/v1/auth/approle/login")))
                .body(payload)
                .send()));
        let decoded: VaultResponse<()> = try!(parse_vault_response(&mut res));
        let token = match decoded.auth {
            Some(ref auth) => auth.client_token.clone(),
            None => {
                return Err(Error::Vault(format!("No client token found in response: `{:?}`",
                                                &decoded.auth)))
            }
        };
        Ok(VaultClient {
            host: host,
            token: token,
            client: client,
            data: Some(decoded),
        })
    }

    /// Construct a `VaultClient` where no lookup is done through vault since it is assumed that the
    /// provided token is a single-use token.
    ///
    /// A common use case for this method is when a `wrapping_token` has been received and you want
    /// to query the `sys/wrapping/unwrap` endpoint.
    pub fn new_no_lookup<U>(host: U, token: &str) -> Result<VaultClient<()>>
        where U: TryInto<Url, Err = Error>
    {
        let client = Client::new()?;
        let host = try!(host.try_into());
        Ok(VaultClient {
            host: host,
            token: token.to_string(),
            client: client,
            data: None,
        })
    }
}

impl<T> VaultClient<T>
    where T: Decodable
{
    /// Renew lease for `VaultClient`'s token and updates the
    /// `self.data.auth` based upon the response.  Corresponds to
    /// [`/auth/token/renew-self`][token].
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let mut client = Client::new(host, token).unwrap();
    ///
    /// client.renew().unwrap();
    /// # }
    /// ```
    ///
    /// [token]: https://www.vaultproject.io/docs/auth/token.html
    pub fn renew(&mut self) -> Result<()> {
        let mut res = try!(self.post("/v1/auth/token/renew-self", None, None));
        let vault_res: VaultResponse<T> = try!(parse_vault_response(&mut res));
        if let Some(ref mut data) = self.data {
            data.auth = vault_res.auth;
        }
        Ok(())
    }

    /// Renew the lease for the specified token.  Requires `root`
    /// privileges.  Corresponds to [`/auth/token/renew[/token]`][token].
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    ///
    /// let token_to_renew = "test12345";
    /// client.renew_token(token_to_renew, None).unwrap();
    /// # }
    /// ```
    ///
    /// [token]: https://www.vaultproject.io/docs/auth/token.html
    pub fn renew_token(&self, token: &str, increment: Option<u64>) -> Result<Auth> {
        let body = try!(json::encode(&RenewOptions { increment: increment }));
        let url = format!("/v1/auth/token/renew/{}", token);
        let mut res = try!(self.post(&url, Some(&body), None));
        let vault_res: VaultResponse<()> = try!(parse_vault_response(&mut res));
        vault_res.auth
            .ok_or_else(|| Error::Vault("No auth data returned while renewing token".to_owned()))
    }

    /// Revoke `VaultClient`'s token. This token can no longer be used.
    /// Corresponds to [`/auth/token/revoke-self`][token].
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::{client, Client};
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    ///
    /// // Create a temporary token, and use it to create a new client.
    /// let opts = client::TokenOptions::default()
    ///   .ttl(client::VaultDuration::minutes(5));
    /// let res = client.create_token(&opts).unwrap();
    /// let mut new_client = Client::new(host, &res.client_token).unwrap();
    ///
    /// // Issue and use a bunch of temporary dynamic credentials.
    ///
    /// // Revoke all our dynamic credentials with a single command.
    /// new_client.revoke().unwrap();
    /// # }
    /// ```
    ///
    /// Note that we consume our `self` parameter, so you cannot use the
    /// client after revoking it.
    ///
    /// [token]: https://www.vaultproject.io/docs/auth/token.html
    pub fn revoke(self) -> Result<()> {
        let _ = try!(self.post("/v1/auth/token/revoke-self", None, None));
        Ok(())
    }

    /// Renew a specific lease that your token controls.  Corresponds to
    /// [`/v1/sys/renew`][renew].
    ///
    /// ```no_run
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    ///
    /// // TODO: Right now, we offer no way to get lease information for a
    /// // secret.
    /// let lease_id: String = unimplemented!();
    ///
    /// client.renew_lease(&lease_id, None).unwrap();
    /// # }
    /// ```
    ///
    /// [renew]: https://www.vaultproject.io/docs/http/sys-renew.html
    pub fn renew_lease(&self, lease_id: &str, increment: Option<u64>) -> Result<VaultResponse<()>> {
        let body = try!(json::encode(&RenewOptions { increment: increment }));
        let mut res = try!(self.put(&format!("/v1/sys/renew/{}", lease_id), Some(&body), None));
        let vault_res: VaultResponse<()> = try!(parse_vault_response(&mut res));
        Ok(vault_res)
    }

    /// Lookup token information for this client's token.  Corresponds to
    /// [`/auth/token/lookup-self`][token].
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    ///
    /// let res = client.lookup().unwrap();
    /// assert!(res.data.unwrap().policies.len() >= 0);
    /// # }
    /// ```
    ///
    /// [token]: https://www.vaultproject.io/docs/auth/token.html
    pub fn lookup(&self) -> Result<VaultResponse<TokenData>> {
        let mut res = try!(self.get("/v1/auth/token/lookup-self", None));
        let vault_res: VaultResponse<TokenData> = try!(parse_vault_response(&mut res));
        Ok(vault_res)
    }

    /// Create a new vault token using the specified options.  Corresponds to
    /// [`/auth/token/create`][token].
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::{client, Client};
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    ///
    /// let opts = client::TokenOptions::default()
    ///   .display_name("test_token")
    ///   .policies(vec!("root"))
    ///   .default_policy(false)
    ///   .orphan(true)
    ///   .renewable(false)
    ///   .display_name("jdoe-temp")
    ///   .number_of_uses(10)
    ///   .ttl(client::VaultDuration::minutes(1))
    ///   .explicit_max_ttl(client::VaultDuration::minutes(3));
    /// let res = client.create_token(&opts).unwrap();
    ///
    /// # let new_client = Client::new(host, &res.client_token).unwrap();
    /// # new_client.revoke().unwrap();
    /// # }
    /// ```
    ///
    /// [token]: https://www.vaultproject.io/docs/auth/token.html
    pub fn create_token(&self, opts: &TokenOptions) -> Result<Auth> {
        let body = try!(json::encode(opts));
        let mut res = try!(self.post("/v1/auth/token/create", Some(&body), None));
        let vault_res: VaultResponse<()> = try!(parse_vault_response(&mut res));
        vault_res.auth.ok_or_else(|| Error::Vault("Created token did not include auth data".into()))
    }

    ///
    /// Saves a secret
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    /// let res = client.set_secret("hello_set", "world");
    /// assert!(res.is_ok());
    /// # }
    /// ```
    pub fn set_secret(&self, key: &str, value: &str) -> Result<()> {
        let _ = try!(self.post(&format!("/v1/secret/{}", key)[..],
                               Some(&format!("{{\"value\": \"{}\"}}", self.escape(value))[..]),
                               None));
        Ok(())
    }

    fn escape(&self, input: &str) -> String {
        input.replace("\n", "\\n")
    }

    ///
    /// Fetches a saved secret
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    /// let res = client.set_secret("hello_get", "world");
    /// assert!(res.is_ok());
    /// let res = client.get_secret("hello_get");
    /// assert!(res.is_ok());
    /// assert_eq!(res.unwrap(), "world");
    /// # }
    /// ```
    pub fn get_secret(&self, key: &str) -> Result<String> {
        let mut res = try!(self.get(&format!("/v1/secret/{}", key)[..], None));
        let decoded: VaultResponse<SecretData> = try!(parse_vault_response(&mut res));
        match decoded.data {
            Some(data) => Ok(data.value),
            _ => Err(Error::Vault(format!("No secret found in response: `{:#?}`", decoded))),
        }
    }

    /// Fetch a wrapped secret. Token (one-time use) to fetch secret will be in `wrap_info.token`
    /// https://www.vaultproject.io/docs/secrets/cubbyhole/index.html
    pub fn get_secret_wrapped(&self, key: &str, wrap_ttl: &str) -> Result<VaultResponse<()>> {
        let mut res = try!(self.get(&format!("/v1/secret/{}", key)[..], Some(wrap_ttl)));
        parse_vault_response(&mut res)
    }

    /// Using a vault client created from a wrapping token, fetch the unwrapped `VaultResponse` from
    /// `sys/wrapping/unwrap`.
    ///
    /// The `data` attribute of `VaultResponse` should contain the unwrapped information, which is
    /// returned as a `HashMap<String, String>`.
    #[cfg(feature = "vault_0.6.2")]
    pub fn get_unwrapped_response(&self) -> Result<VaultResponse<HashMap<String, String>>> {
        let mut res = try!(self.post("/v1/sys/wrapping/unwrap", None, None));
        parse_vault_response(&mut res)
    }

    /// Reads the properties of an existing `AppRole`.
    #[cfg(feature = "vault_0.6.1")]
    pub fn get_app_role_properties(&self,
                                   role_name: &str)
                                   -> Result<VaultResponse<AppRoleProperties>> {
        let mut res = try!(self.get(&format!("/v1/auth/approle/role/{}", role_name), None));
        parse_vault_response(&mut res)
    }

    /// This function is an "escape hatch" of sorts to call any other vault api methods that
    /// aren't directly supported in this library.
    ///
    /// Select the http verb you want, along with the endpoint, e.g. `auth/token/create`, along
    /// with any wrapping or associated body text and the request will be sent.
    ///
    /// See `it_can_perform_approle_workflow` test case for examples.
    pub fn call_endpoint<D: Decodable>(&self,
                                       http_verb: HttpVerb,
                                       endpoint: &str,
                                       wrap_ttl: Option<&str>,
                                       body: Option<&str>)
                                       -> Result<EndpointResponse<D>> {
        let url = format!("/v1/{}", endpoint);
        match http_verb {
            HttpVerb::GET => {
                let mut res = try!(self.get(&url, wrap_ttl));
                parse_endpoint_response(&mut res)
            }
            HttpVerb::POST => {
                let mut res = try!(self.post(&url, body, wrap_ttl));
                parse_endpoint_response(&mut res)
            }
            HttpVerb::PUT => {
                let mut res = try!(self.put(&url, body, wrap_ttl));
                parse_endpoint_response(&mut res)
            }
            HttpVerb::DELETE => {
                let mut res = try!(self.delete(&url));
                parse_endpoint_response(&mut res)
            }
            HttpVerb::LIST => {
                let mut res = try!(self.list(&url, body, wrap_ttl));
                parse_endpoint_response(&mut res)
            }
        }
    }

    /// Accesses a given endpoint using the provided `wrap_ttl` and returns a single-use
    /// `wrapping_token` to access the response provided by the endpoint.
    pub fn get_wrapping_token_for_endpoint(&self,
                                           http_verb: HttpVerb,
                                           endpoint: &str,
                                           wrap_ttl: &str,
                                           body: Option<&str>)
                                           -> Result<String> {
        let res = try!(self.call_endpoint::<()>(http_verb, endpoint, Some(wrap_ttl), body));
        match res {
            EndpointResponse::VaultResponse(res) => {
                match res.wrap_info {
                    Some(wrap_info) => Ok(wrap_info.token),
                    _ => Err(Error::Vault(format!("wrap_info is missing in response: {:?}", res))),
                }
            }
            EndpointResponse::Empty => Err(Error::Vault("Received an empty response".to_string())),
        }
    }

    ///
    /// Deletes a saved secret
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    /// let res = client.set_secret("hello_delete", "world");
    /// assert!(res.is_ok());
    /// let res = client.delete_secret("hello_delete");
    /// assert!(res.is_ok());
    /// # }
    /// ```
    pub fn delete_secret(&self, key: &str) -> Result<()> {
        let _ = try!(self.delete(&format!("/v1/secret/{}", key)[..]));
        Ok(())
    }

    /// Get postgresql secret backend
    /// https://www.vaultproject.io/docs/secrets/postgresql/index.html
    pub fn get_postgresql_backend(&self, name: &str) -> Result<VaultResponse<PostgresqlLogin>> {
        let mut res = try!(self.get(&format!("/v1/postgresql/creds/{}", name)[..], None));
        let decoded: VaultResponse<PostgresqlLogin> = try!(parse_vault_response(&mut res));
        Ok(decoded)
    }

    /// Get a list of policy names defined by this vault.  This requires
    /// `root` privileges. Corresponds to [`/sys/policy`][/sys/policy].
    ///
    /// ```
    /// # extern crate hashicorp_vault as vault;
    /// # use vault::Client;
    /// # fn main() {
    /// let host = "http://127.0.0.1:8200";
    /// let token = "test12345";
    /// let client = Client::new(host, token).unwrap();
    ///
    /// let res = client.policies().unwrap();
    /// assert!(res.contains(&"root".to_owned()));
    /// # }
    /// ```
    ///
    /// [/sys/policy]: https://www.vaultproject.io/docs/http/sys-policy.html
    pub fn policies(&self) -> Result<Vec<String>> {
        let mut res = try!(self.get("/v1/sys/policy", None));
        let decoded: PoliciesResponse = try!(parse_vault_response(&mut res));
        Ok(decoded.policies)
    }

    fn get(&self, endpoint: &str, wrap_ttl: Option<&str>) -> Result<Response> {
        let mut req = self.client
            .get(try!(self.host.join(endpoint)))
            .header(XVaultToken(self.token.to_string()))
            .header(header::ContentType::json());
        if let Some(wrap_ttl) = wrap_ttl {
            req = req.header(XVaultWrapTTL(wrap_ttl.into()));
        }

        Ok(try!(handle_hyper_response(req.send())))
    }

    fn delete(&self, endpoint: &str) -> Result<Response> {
        Ok(try!(handle_hyper_response(self.client
            .request(Method::Delete, try!(self.host.join(endpoint)))
            .header(XVaultToken(self.token.to_string()))
            .header(header::ContentType::json())
            .send())))
    }

    fn post(&self, endpoint: &str, body: Option<&str>, wrap_ttl: Option<&str>) -> Result<Response> {
        let mut req = self.client
            .post(try!(self.host.join(endpoint)))
            .header(XVaultToken(self.token.to_string()))
            .header(header::ContentType::json());
        if let Some(wrap_ttl) = wrap_ttl {
            req = req.header(XVaultWrapTTL(wrap_ttl.into()));
        }
        if let Some(body) = body {
            req = req.body(body);
        }

        Ok(try!(handle_hyper_response(req.send())))
    }

    fn put(&self, endpoint: &str, body: Option<&str>, wrap_ttl: Option<&str>) -> Result<Response> {
        let mut req = self.client
            .request(Method::Put, try!(self.host.join(endpoint)))
            .header(XVaultToken(self.token.to_string()))
            .header(header::ContentType::json());
        if let Some(wrap_ttl) = wrap_ttl {
            req = req.header(XVaultWrapTTL(wrap_ttl.into()));
        }
        if let Some(body) = body {
            req = req.body(body);
        }

        Ok(try!(handle_hyper_response(req.send())))
    }

    fn list(&self, endpoint: &str, body: Option<&str>, wrap_ttl: Option<&str>) -> Result<Response> {
        // let method = hyper::method::Method::Extension("LIST".into());
        let mut req = self.client
            .request(Method::Extension("LIST".into()),
                     try!(self.host.join(endpoint)))
            .header(XVaultToken(self.token.to_string()))
            .header(header::ContentType::json());
        if let Some(wrap_ttl) = wrap_ttl {
            req = req.header(XVaultWrapTTL(wrap_ttl.into()));
        }
        if let Some(body) = body {
            req = req.body(body);
        }

        Ok(try!(handle_hyper_response(req.send())))
    }
}

/// helper fn to check `Response` for success
fn handle_hyper_response(res: ::std::result::Result<Response, reqwest::Error>) -> Result<Response> {
    let mut res = try!(res);
    if res.status().is_success() {
        Ok(res)
    } else {
        let mut error_msg = String::new();
        let _ = res.read_to_string(&mut error_msg).unwrap_or({
            error_msg.push_str("Could not read vault response.");
            0
        });
        Err(Error::Vault(format!("Vault request failed: {:?}, error message: `{}`",
                                 res,
                                 error_msg)))
    }
}

fn parse_vault_response<T>(res: &mut Response) -> Result<T>
    where T: Decodable
{
    let mut body = String::new();
    let _ = try!(res.read_to_string(&mut body));
    trace!("Response: {:?}", &body);
    let vault_res: T = try!(json::decode(&body));
    Ok(vault_res)
}

/// checks if response is empty before attempting to convert to a `VaultResponse`
fn parse_endpoint_response<T>(res: &mut Response) -> Result<EndpointResponse<T>>
    where T: Decodable
{
    let mut body = String::new();
    let _ = try!(res.read_to_string(&mut body));
    trace!("Response: {:?}", &body);
    if body.is_empty() {
        Ok(EndpointResponse::Empty)
    } else {
        let vault_res: VaultResponse<T> = try!(json::decode(&body));
        Ok(EndpointResponse::VaultResponse(vault_res))
    }
}
