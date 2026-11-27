use std::env;

#[derive(Clone, Copy, Debug, PartialEq, Eq)]
pub struct SignupLimits {
    pub(super) per_ip_hour: u32,
    pub(super) per_ip_day: u32,
    pub(super) global_day: u32,
}

impl SignupLimits {
    pub(super) fn from_env() -> Option<Self> {
        Self::load(|key| env::var(key)).unwrap_or_else(|key| panic!("Invalid {key}"))
    }

    fn load(
        mut read: impl FnMut(&str) -> Result<String, env::VarError>,
    ) -> Result<Option<Self>, &'static str> {
        const SWITCH: &str = "PUBLIC_SIGNUP_LIMITS_ENABLED";
        match read(SWITCH).as_deref() {
            Ok("true") => {}
            Ok("false") | Err(env::VarError::NotPresent) => return Ok(None),
            _ => return Err(SWITCH),
        }
        let mut limit = |key, default| match read(key) {
            Ok(value) => value.parse::<u32>().ok().filter(|n| *n > 0).ok_or(key),
            Err(env::VarError::NotPresent) => Ok(default),
            Err(_) => Err(key),
        };
        Ok(Some(Self {
            per_ip_hour: limit("PUBLIC_SIGNUP_LIMIT_PER_IP_HOUR", 5)?,
            per_ip_day: limit("PUBLIC_SIGNUP_LIMIT_PER_IP_DAY", 20)?,
            global_day: limit("PUBLIC_SIGNUP_LIMIT_GLOBAL_DAY", 500)?,
        }))
    }

    #[cfg(test)]
    pub(crate) fn for_test(per_ip_hour: u32, per_ip_day: u32, global_day: u32) -> Self {
        Self {
            per_ip_hour,
            per_ip_day,
            global_day,
        }
    }

    pub(crate) fn caps(self) -> [u32; 3] {
        [self.per_ip_hour, self.per_ip_day, self.global_day]
    }
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn signup_limits_default_off_and_parse_approved_limits() {
        assert_eq!(
            SignupLimits::load(|_| Err(env::VarError::NotPresent)),
            Ok(None)
        );
        let config = |overrides: &[(&str, &str)]| {
            SignupLimits::load(|key| {
                overrides
                    .iter()
                    .find(|(name, _)| *name == key)
                    .map(|(_, value)| (*value).to_owned())
                    .ok_or(env::VarError::NotPresent)
            })
        };
        let on = ("PUBLIC_SIGNUP_LIMITS_ENABLED", "true");
        assert_eq!(config(&[on]).unwrap().unwrap().caps(), [5, 20, 500]);
        assert_eq!(
            config(&[on, ("PUBLIC_SIGNUP_LIMIT_GLOBAL_DAY", "42")])
                .unwrap()
                .unwrap()
                .caps(),
            [5, 20, 42],
        );
        assert_eq!(
            config(&[on, ("PUBLIC_SIGNUP_LIMIT_PER_IP_DAY", "0")]),
            Err("PUBLIC_SIGNUP_LIMIT_PER_IP_DAY"),
        );
    }
}
