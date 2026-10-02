use super::*;
use futures::StreamExt;
use reqwest::Method;

/// Fixed local checks, without Docker response bodies, names or credentials.
#[derive(Clone, Copy, Debug, thiserror::Error)]
pub(super) enum CheckFailure {
    #[error("docker_socket_unavailable")]
    DockerSocketUnavailable,
    #[error("container_not_found")]
    ContainerNotFound,
    #[error("invalid_container_name")]
    InvalidContainerName,
    #[error("source_name_changed")]
    SourceNameChanged,
    #[error("source_not_official_image")]
    SourceNotOfficialImage,
    #[error("source_auto_remove")]
    SourceAutoRemove,
    #[error("source_owner_mismatch")]
    SourceOwnerMismatch,
    #[error("source_version_unknown")]
    SourceVersionUnknown,
    #[error("companion_name_taken")]
    CompanionNameTaken,
    #[error("update_volume_mismatch")]
    UpdateVolumeMismatch,
    #[error("downgrade_refused")]
    DowngradeRefused,
    #[error("invalid_target_version")]
    InvalidTargetVersion,
}
impl CheckFailure {
    pub(super) fn code(self) -> &'static str {
        match self {
            Self::DockerSocketUnavailable => "docker_socket_unavailable",
            Self::ContainerNotFound => "container_not_found",
            Self::InvalidContainerName => "invalid_container_name",
            Self::SourceNameChanged => "source_name_changed",
            Self::SourceNotOfficialImage => "source_not_official_image",
            Self::SourceAutoRemove => "source_auto_remove",
            Self::SourceOwnerMismatch => "source_owner_mismatch",
            Self::SourceVersionUnknown => "source_version_unknown",
            Self::CompanionNameTaken => "companion_name_taken",
            Self::UpdateVolumeMismatch => "update_volume_mismatch",
            Self::DowngradeRefused => "downgrade_refused",
            Self::InvalidTargetVersion => "invalid_target_version",
        }
    }
}

fn transport_failure(error: reqwest::Error) -> anyhow::Error {
    if error.is_connect() {
        CheckFailure::DockerSocketUnavailable.into()
    } else {
        error.into()
    }
}

pub struct Docker {
    client: reqwest::Client,
    #[cfg(test)]
    test_endpoint: Option<String>,
    #[cfg(test)]
    test_digest: Option<Result<String, &'static str>>,
    #[cfg(test)]
    pub(super) test_image: Option<String>,
    #[cfg(test)]
    pub(super) test_failed_replacement: bool,
}

impl Docker {
    pub fn new() -> Result<Self> {
        Ok(Self {
            #[cfg(test)]
            test_endpoint: None,
            #[cfg(test)]
            test_digest: None,
            #[cfg(test)]
            test_image: None,
            #[cfg(test)]
            test_failed_replacement: false,
            client: reqwest::Client::builder()
                .unix_socket("/var/run/docker.sock")
                .no_proxy()
                .timeout(Duration::from_secs(30))
                .build()?,
        })
    }

    fn endpoint(&self) -> &str {
        #[cfg(test)]
        if let Some(endpoint) = &self.test_endpoint {
            return endpoint;
        }
        "http://localhost"
    }
    #[cfg(test)]
    pub(super) fn fixture(endpoint: String, verification: Result<String, &'static str>) -> Self {
        Self {
            client: reqwest::Client::builder().no_proxy().build().unwrap(),
            test_endpoint: Some(endpoint),
            test_digest: Some(verification),
            test_image: None,
            test_failed_replacement: false,
        }
    }

    #[cfg(test)]
    pub(super) fn local_fixture(image: &str) -> Result<Self> {
        let mut api = Self::new()?;
        api.test_image = Some(image.into());
        api.test_digest = Some(Ok(format!("sha256:{}", "a".repeat(64))));
        Ok(api)
    }

    pub(super) async fn call(
        &self,
        method: Method,
        path: &str,
        body: Option<&Value>,
    ) -> Result<Value> {
        #[cfg(test)]
        let operation = format!("{method} {path}");
        let already_in_state =
            method == Method::POST && (path.ends_with("/start") || path.contains("/stop?"));
        let inspecting =
            method == Method::GET && path.starts_with("/containers/") && path.ends_with("/json");
        let mut request = self
            .client
            .request(method, format!("{}/v1.45{path}", self.endpoint()));
        if let Some(body) = body {
            request = request.json(body);
        }
        let mut response = request.send().await.map_err(transport_failure)?;
        if inspecting && response.status() == reqwest::StatusCode::NOT_FOUND {
            return Err(CheckFailure::ContainerNotFound.into());
        }
        #[cfg(test)]
        if !response.status().is_success() {
            eprintln!("Docker test operation {operation}: {}", response.status());
        }
        ensure!(
            response.status().is_success()
                || (already_in_state && response.status() == reqwest::StatusCode::NOT_MODIFIED),
            "Docker operation refused ({})",
            response.status().as_u16()
        );
        let mut bytes = Vec::new();
        while let Some(chunk) = response.chunk().await? {
            ensure!(
                bytes.len() + chunk.len() <= 4 * 1024 * 1024,
                "Docker metadata exceeded limit"
            );
            bytes.extend_from_slice(&chunk);
        }
        if bytes.is_empty() {
            Ok(Value::Null)
        } else {
            Ok(serde_json::from_slice(&bytes)?)
        }
    }

    pub async fn inspect(&self, name: &str) -> Result<Value> {
        self.call(
            Method::GET,
            &format!("/containers/{}/json", urlencoding::encode(name)),
            None,
        )
        .await
    }
    pub(super) async fn image_id(&self, image: &str) -> Result<String> {
        #[cfg(test)]
        let image = self.test_image.as_deref().unwrap_or(image);
        let value = self
            .call(
                Method::GET,
                &format!("/images/{}/json", urlencoding::encode(image)),
                None,
            )
            .await?;
        Ok(value["Id"]
            .as_str()
            .context("Missing image identity")?
            .into())
    }
    pub async fn stop(&self, id: &str) -> Result<()> {
        self.call(
            Method::POST,
            &format!("/containers/{}/stop?t=10", urlencoding::encode(id)),
            None,
        )
        .await?;
        Ok(())
    }
    pub async fn start(&self, id: &str) -> Result<()> {
        self.call(
            Method::POST,
            &format!("/containers/{}/start", urlencoding::encode(id)),
            None,
        )
        .await?;
        Ok(())
    }
    pub async fn rename(&self, id: &str, name: &str) -> Result<()> {
        self.call(
            Method::POST,
            &format!(
                "/containers/{}/rename?name={}",
                urlencoding::encode(id),
                urlencoding::encode(name)
            ),
            None,
        )
        .await?;
        Ok(())
    }
    pub async fn remove(&self, id: &str) -> Result<()> {
        let response = self
            .client
            .delete(format!(
                "{}/v1.45/containers/{}?force=true&v=false",
                self.endpoint(),
                urlencoding::encode(id)
            ))
            .send()
            .await
            .map_err(transport_failure)?;
        // A committed update may have removed the retained container before a
        // crash. Cleanup must be idempotent, without hiding other Docker errors.
        ensure!(
            response.status().is_success() || response.status() == reqwest::StatusCode::NOT_FOUND,
            "Docker removal refused ({})",
            response.status().as_u16()
        );
        Ok(())
    }
    pub async fn create(&self, name: &str, config: &Value) -> Result<String> {
        #[cfg(test)]
        let config = &{
            let mut config = config.clone();
            if let Some(image) = &self.test_image {
                config["Image"] = json!(image);
            }
            if self.test_failed_replacement {
                config["Entrypoint"] = json!(["/bin/false"]);
            }
            config
        };
        let result = self
            .call(
                Method::POST,
                &format!("/containers/create?name={}", urlencoding::encode(name)),
                Some(config),
            )
            .await?;
        Ok(result["Id"]
            .as_str()
            .context("Docker omitted container identity")?
            .into())
    }
    pub async fn disconnect_networks(&self, id: &str, networks: &Value) -> Result<()> {
        if let Some(networks) = networks.as_object() {
            for name in networks
                .keys()
                .filter(|name| !matches!(name.as_str(), "host" | "none"))
            {
                self.call(
                    Method::POST,
                    &format!("/networks/{}/disconnect", urlencoding::encode(name)),
                    Some(&json!({"Container":id,"Force":true})),
                )
                .await?;
            }
        }
        Ok(())
    }
    pub async fn restore_networks(&self, id: &str, networks: &Value) -> Result<()> {
        let current = self.inspect(id).await?;
        if let Some(networks) = networks.as_object() {
            for (name, endpoint) in networks {
                if current["NetworkSettings"]["Networks"].get(name).is_none() {
                    self.call(
                        Method::POST,
                        &format!("/networks/{}/connect", urlencoding::encode(name)),
                        Some(&json!({"Container":id,"EndpointConfig":endpoint})),
                    )
                    .await?;
                }
            }
        }
        Ok(())
    }
    pub async fn verified_digest(
        &self,
        image: &str,
        target: &str,
        datastore: Option<&Path>,
    ) -> Result<String> {
        ensure!(
            [update::MACHINE_IMAGE, update::UPDATER_IMAGE].contains(&image),
            "Only official NyxID images can be installed"
        );
        update::version(target).map_err(anyhow::Error::msg)?;
        #[cfg(test)]
        if let Some(result) = &self.test_digest {
            return result.clone().map_err(anyhow::Error::msg);
        }

        let descriptor = self
            .call(
                Method::GET,
                &format!(
                    "/distribution/{}/json",
                    urlencoding::encode(&format!("{image}:{target}"))
                ),
                None,
            )
            .await?;
        let digest = descriptor["Descriptor"]["digest"]
            .as_str()
            .context("Image tag has no digest")?;
        valid_digest(digest)?;
        // The tag is resolved once; both verification and pull use that digest.
        let client = github_client()?;
        crate::update_attestation::verify_image_attestation(
            &client,
            digest.trim_start_matches("sha256:"),
            target,
            datastore,
        )
        .await?;
        Ok(digest.into())
    }
    pub async fn pull(&self, image: &str) -> Result<()> {
        #[cfg(test)]
        if self.test_image.is_some() {
            return Ok(());
        }
        let response = self
            .client
            .post(format!("{}/v1.45/images/create", self.endpoint()))
            .query(&[("fromImage", image)])
            .timeout(Duration::from_secs(900))
            .send()
            .await
            .map_err(transport_failure)?
            .error_for_status()?;
        let mut stream = response.bytes_stream();
        let mut line = Vec::new();
        while let Some(chunk) = stream.next().await {
            for byte in chunk? {
                if byte == b'\n' {
                    if !line.is_empty() {
                        let status: Value = serde_json::from_slice(&line)?;
                        ensure!(status.get("error").is_none(), "Image download failed");
                        line.clear();
                    }
                } else {
                    line.push(byte);
                    ensure!(line.len() <= 65536, "Image progress exceeded limit");
                }
            }
        }
        Ok(())
    }
}

pub fn github_client() -> Result<reqwest::Client> {
    Ok(reqwest::Client::builder()
        .https_only(true)
        .no_proxy()
        .user_agent("nyxid-machine-updater")
        .timeout(Duration::from_secs(30))
        .redirect(reqwest::redirect::Policy::custom(|attempt| {
            if allowed_https(attempt.url()) && attempt.previous().len() < 5 {
                attempt.follow()
            } else {
                attempt.error("Updater redirect host refused")
            }
        }))
        .build()?)
}
fn allowed_https(url: &url::Url) -> bool {
    url.scheme() == "https"
        && url.port_or_known_default() == Some(443)
        && matches!(
            url.host_str(),
            Some(
                "api.github.com"
                    | "github.com"
                    | "raw.githubusercontent.com"
                    | "release-assets.githubusercontent.com"
                    | "objects.githubusercontent.com"
            )
        )
}
pub fn valid_name(name: &str) -> Result<()> {
    ensure!(
        update::container_name(name),
        CheckFailure::InvalidContainerName
    );
    Ok(())
}
fn valid_digest(digest: &str) -> Result<()> {
    ensure!(
        digest.starts_with("sha256:")
            && digest.len() == 71
            && digest[7..].bytes().all(|b| b.is_ascii_hexdigit()),
        "Invalid image digest"
    );
    Ok(())
}
pub fn source_version(inspect: &Value) -> Result<&str> {
    let image = inspect["Config"]["Image"]
        .as_str()
        .ok_or(CheckFailure::SourceVersionUnknown)?;
    if let Some(target) = image.strip_prefix(&format!("{}:", update::MACHINE_IMAGE)) {
        update::version(target).map_err(|_| CheckFailure::SourceVersionUnknown)?;
        return Ok(target);
    }
    let target = inspect["Config"]["Labels"]["dev.nyxid.machine.version"]
        .as_str()
        .ok_or(CheckFailure::SourceVersionUnknown)?;
    update::version(target).map_err(|_| CheckFailure::SourceVersionUnknown)?;
    Ok(target)
}
pub fn validate_source(inspect: &Value, name: &str, migration: bool) -> Result<()> {
    valid_name(name)?;
    ensure!(
        inspect["Name"] == format!("/{name}"),
        CheckFailure::SourceNameChanged
    );
    let image = inspect["Config"]["Image"]
        .as_str()
        .ok_or(CheckFailure::SourceNotOfficialImage)?;
    ensure!(
        image.starts_with(&format!("{}:", update::MACHINE_IMAGE))
            || image.starts_with(&format!("{}@sha256:", update::MACHINE_IMAGE)),
        CheckFailure::SourceNotOfficialImage
    );
    ensure!(
        migration || inspect["Config"]["Labels"][update::CONTAINER_LABEL] == name,
        CheckFailure::SourceOwnerMismatch
    );
    ensure!(
        inspect["HostConfig"]["AutoRemove"] != true,
        CheckFailure::SourceAutoRemove
    );
    Ok(())
}

pub(super) fn validate_target(current: &str, target: &str, rollback: bool) -> Result<()> {
    let current = update::version(current).map_err(|_| CheckFailure::SourceVersionUnknown)?;
    let target = update::version(target).map_err(|_| CheckFailure::InvalidTargetVersion)?;
    ensure!(
        target >= current || rollback,
        CheckFailure::DowngradeRefused
    );
    Ok(())
}

pub fn replacement_config(
    inspect: &Value,
    digest: &str,
    name: &str,
    target: &str,
    migration: bool,
) -> Result<Value> {
    valid_digest(digest)?;
    let mut config = copy_config(inspect)?;
    config["Image"] = json!(format!("{}@{digest}", update::MACHINE_IMAGE));
    if config["Labels"].is_null() {
        config["Labels"] = json!({});
    }
    config["Labels"]["dev.nyxid.machine.version"] = json!(target);
    if migration {
        ensure!(
            !inspect["Mounts"]
                .as_array()
                .context("Missing mount metadata")?
                .iter()
                .any(|m| m["Destination"] == update::UPDATE_VOLUME),
            "An update volume is already attached; repair its companion instead"
        );
        config["HostConfig"]["Binds"]
            .as_array_mut()
            .unwrap()
            .push(json!(format!(
                "{name}-nyxid-update:{}",
                update::UPDATE_VOLUME
            )));
        config["Labels"][update::CONTAINER_LABEL] = json!(name);
    }
    Ok(config)
}

/// Preserve owner configuration, including resolved anonymous volumes. Runtime
/// network IDs are not create parameters; configured endpoint options are.
pub(super) fn copy_config(inspect: &Value) -> Result<Value> {
    let mut config = inspect["Config"].clone();
    ensure!(config.is_object(), "Missing container config");
    config["HostConfig"] = inspect["HostConfig"].clone();
    // Pin anonymous mounts by their resolved volume name too. Docker otherwise
    // allocates fresh anonymous volumes and silently loses the node's identity.
    let mounts = inspect["Mounts"]
        .as_array()
        .context("Missing mount metadata")?;
    let host = config["HostConfig"]
        .as_object_mut()
        .context("Missing host config")?;
    let mut binds = host
        .get("Binds")
        .and_then(Value::as_array)
        .cloned()
        .unwrap_or_default();
    for mount in mounts.iter().filter(|m| m["Type"] == "volume") {
        let destination = mount["Destination"]
            .as_str()
            .context("Missing mount path")?;
        let already = binds.iter().any(|b| {
            b.as_str()
                .is_some_and(|b| b.split(':').nth(1) == Some(destination))
        }) || host
            .get("Mounts")
            .and_then(Value::as_array)
            .is_some_and(|m| m.iter().any(|m| m["Target"] == destination));
        if !already {
            binds.push(json!(format!(
                "{}:{}:{}",
                mount["Name"].as_str().context("Missing volume name")?,
                destination,
                if mount["RW"] == true { "rw" } else { "ro" }
            )));
        }
    }
    config["HostConfig"]["Binds"] = json!(binds);
    // Preserve configured network endpoint settings, excluding daemon runtime IDs.
    let mut networks = serde_json::Map::new();
    if let Some(endpoints) = inspect["NetworkSettings"]["Networks"].as_object() {
        for (name, endpoint) in endpoints {
            let mut retained = serde_json::Map::new();
            for key in ["IPAMConfig", "Links", "Aliases", "DriverOpts", "GwPriority"] {
                if let Some(value) = endpoint.get(key) {
                    retained.insert(key.into(), value.clone());
                }
            }
            networks.insert(name.clone(), Value::Object(retained));
        }
    }
    // Hostname/DNS/ports are owned by the shared network namespace. Inspect
    // materializes its hostname, but Docker rejects supplying it on recreation.
    if config["HostConfig"]["NetworkMode"]
        .as_str()
        .is_some_and(|mode| mode.starts_with("container:"))
    {
        for key in ["Hostname", "Domainname"] {
            config.as_object_mut().unwrap().remove(key);
        }
    }
    config["NetworkingConfig"] = json!({"EndpointsConfig":networks});
    Ok(config)
}

#[cfg(test)]
mod tests {
    use super::*;
    #[test]
    fn replacement_preserves_config_and_resolved_anonymous_volumes() {
        let inspect = json!({"Config":{"Image":format!("{}:0.40.0",update::MACHINE_IMAGE),"Env":["PRIVATE=value"],"Cmd":["node","start"],"Labels":{"custom":"keep"},"User":"0"},"HostConfig":{"SecurityOpt":["seccomp=custom"],"ShmSize":268435456,"RestartPolicy":{"Name":"unless-stopped"},"Binds":["identity:/identity"]},"Mounts":[{"Type":"volume","Name":"anonymous-workspace","Destination":"/workspace","RW":true}],"NetworkSettings":{"Networks":{"bridge":{"IPAMConfig":null,"Aliases":["machine"]}}}});
        let result = replacement_config(
            &inspect,
            &format!("sha256:{}", "a".repeat(64)),
            "test-machine",
            "0.41.0",
            true,
        )
        .unwrap();
        for key in ["Env", "Cmd", "User"] {
            assert_eq!(result[key], inspect["Config"][key]);
        }
        for key in ["SecurityOpt", "ShmSize", "RestartPolicy"] {
            assert_eq!(result["HostConfig"][key], inspect["HostConfig"][key]);
        }
        assert!(
            result["HostConfig"]["Binds"]
                .as_array()
                .unwrap()
                .contains(&json!("anonymous-workspace:/workspace:rw"))
        );
        assert_eq!(result["Labels"]["custom"], "keep");
        assert_eq!(result["Labels"][update::CONTAINER_LABEL], "test-machine");
    }
    #[test]
    fn unrelated_images_containers_and_destinations_are_refused() {
        let bad = json!({"Name":"/machine","Config":{"Image":"attacker/image:0.41.0","Labels":{}}});
        assert!(validate_source(&bad, "machine", true).is_err());
        for bad in ["../machine", "machine;id", "-name", ""] {
            assert!(valid_name(bad).is_err());
        }
        for url in [
            "http://github.com/a",
            "https://github.com.attacker.test/a",
            "https://localhost/a",
        ] {
            assert!(!allowed_https(&url.parse().unwrap()));
        }
    }
}
