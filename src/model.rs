use serde_json::Value;

#[derive(Clone, Copy, Debug, Default, PartialEq)]
pub struct Vec2 {
    pub x: f64,
    pub y: f64,
}

impl Vec2 {
    pub fn length(self) -> f64 {
        self.x.hypot(self.y)
    }

    pub fn distance(self, other: Self) -> f64 {
        (self.x - other.x).hypot(self.y - other.y)
    }

    pub fn clamp(self, maximum: f64) -> Self {
        let length = self.length();
        if length > maximum && length > 0.0 {
            self * (maximum / length)
        } else {
            self
        }
    }
}

impl std::ops::Add for Vec2 {
    type Output = Self;
    fn add(self, rhs: Self) -> Self {
        Self {
            x: self.x + rhs.x,
            y: self.y + rhs.y,
        }
    }
}

impl std::ops::Sub for Vec2 {
    type Output = Self;
    fn sub(self, rhs: Self) -> Self {
        Self {
            x: self.x - rhs.x,
            y: self.y - rhs.y,
        }
    }
}

impl std::ops::Mul<f64> for Vec2 {
    type Output = Self;
    fn mul(self, rhs: f64) -> Self {
        Self {
            x: self.x * rhs,
            y: self.y * rhs,
        }
    }
}

#[derive(Clone, Debug)]
pub struct Transport {
    pub id: String,
    pub position: Vec2,
    pub velocity: Vec2,
    pub self_acceleration: Vec2,
    pub anomaly_acceleration: Vec2,
    pub alive: bool,
}

#[derive(Clone, Debug)]
pub struct Anomaly {
    pub position: Vec2,
    pub velocity: Vec2,
    pub core_radius: f64,
    pub effect_radius: f64,
    /// Positive attracts; negative repels in the external Desert contract.
    pub strength: f64,
}

#[derive(Clone, Debug)]
pub struct Bounty {
    pub position: Vec2,
    pub points: f64,
    // API coin radius; collection uses the swept combined radius transportRadius + radius.
    #[allow(dead_code)]
    pub radius: f64,
}

#[derive(Clone, Debug)]
pub struct MovingBody {
    pub position: Vec2,
    pub velocity: Vec2,
    pub alive: bool,
}

#[derive(Clone, Debug)]
pub struct Desert {
    pub errors: Vec<String>,
    pub map_size: Vec2,
    pub max_accel: f64,
    pub max_speed: f64,
    pub transport_radius: f64,
    pub transports: Vec<Transport>,
    pub anomalies: Vec<Anomaly>,
    pub bounties: Vec<Bounty>,
    pub enemies: Vec<MovingBody>,
}

#[derive(Clone, Debug)]
pub struct Command {
    pub id: String,
    pub acceleration: Vec2,
}

impl Desert {
    pub fn parse(value: Value) -> Result<Self, String> {
        let root = value.as_object().ok_or("Desert must be a JSON object")?;
        let transports = root
            .get("transports")
            .and_then(Value::as_array)
            .ok_or("Desert.transports must be an array")?
            .iter()
            .map(parse_transport)
            .collect::<Result<Vec<_>, _>>()?;
        let anomalies = array(root, "anomalies")?
            .iter()
            .map(parse_anomaly)
            .collect::<Result<Vec<_>, _>>()?;
        let bounties = array(root, "bounties")?
            .iter()
            .map(parse_bounty)
            .collect::<Result<Vec<_>, _>>()?;
        let enemies = array(root, "enemies")?
            .iter()
            .map(parse_body)
            .collect::<Result<Vec<_>, _>>()?;
        let errors = array(root, "errors")?
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_string)
            .collect();
        Ok(Self {
            errors,
            map_size: vector(root.get("mapSize").ok_or("Desert.mapSize is missing")?)?,
            max_accel: number(root, "maxAccel", 40.0),
            max_speed: number(root, "maxSpeed", 110.0),
            transport_radius: number(root, "transportRadius", 5.0),
            transports,
            anomalies,
            bounties,
            enemies,
        })
    }
}

fn array<'a>(
    root: &'a serde_json::Map<String, Value>,
    key: &str,
) -> Result<&'a Vec<Value>, String> {
    root.get(key)
        .and_then(Value::as_array)
        .ok_or_else(|| format!("Desert.{key} must be an array"))
}

fn number(root: &serde_json::Map<String, Value>, key: &str, fallback: f64) -> f64 {
    root.get(key)
        .and_then(Value::as_f64)
        .filter(|n| n.is_finite())
        .unwrap_or(fallback)
}

fn vector(value: &Value) -> Result<Vec2, String> {
    let object = value.as_object().ok_or("vector must be an object")?;
    let x = object
        .get("x")
        .and_then(Value::as_f64)
        .ok_or("vector.x must be a number")?;
    let y = object
        .get("y")
        .and_then(Value::as_f64)
        .ok_or("vector.y must be a number")?;
    if !x.is_finite() || !y.is_finite() {
        return Err("vector components must be finite".into());
    }
    Ok(Vec2 { x, y })
}

fn parse_transport(value: &Value) -> Result<Transport, String> {
    let object = value.as_object().ok_or("transport must be an object")?;
    Ok(Transport {
        id: object
            .get("id")
            .and_then(Value::as_str)
            .ok_or("transport.id is missing")?
            .to_string(),
        position: Vec2 {
            x: object
                .get("x")
                .and_then(Value::as_f64)
                .ok_or("transport.x is missing")?,
            y: object
                .get("y")
                .and_then(Value::as_f64)
                .ok_or("transport.y is missing")?,
        },
        velocity: vector(
            object
                .get("velocity")
                .ok_or("transport.velocity is missing")?,
        )?,
        self_acceleration: vector(
            object
                .get("selfAcceleration")
                .ok_or("transport.selfAcceleration is missing")?,
        )?,
        anomaly_acceleration: vector(
            object
                .get("anomalyAcceleration")
                .ok_or("transport.anomalyAcceleration is missing")?,
        )?,
        alive: object.get("status").and_then(Value::as_str) == Some("alive")
            && object
                .get("health")
                .and_then(Value::as_f64)
                .unwrap_or(100.0)
                > 0.0,
    })
}

fn parse_anomaly(value: &Value) -> Result<Anomaly, String> {
    let object = value.as_object().ok_or("anomaly must be an object")?;
    Ok(Anomaly {
        position: Vec2 {
            x: object
                .get("x")
                .and_then(Value::as_f64)
                .ok_or("anomaly.x is missing")?,
            y: object
                .get("y")
                .and_then(Value::as_f64)
                .ok_or("anomaly.y is missing")?,
        },
        velocity: vector(
            object
                .get("velocity")
                .ok_or("anomaly.velocity is missing")?,
        )?,
        core_radius: object.get("radius").and_then(Value::as_f64).unwrap_or(0.0),
        effect_radius: object
            .get("effectiveRadius")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
        strength: object
            .get("strength")
            .and_then(Value::as_f64)
            .unwrap_or(0.0),
    })
}

fn parse_bounty(value: &Value) -> Result<Bounty, String> {
    let object = value.as_object().ok_or("bounty must be an object")?;
    Ok(Bounty {
        position: Vec2 {
            x: object
                .get("x")
                .and_then(Value::as_f64)
                .ok_or("bounty.x is missing")?,
            y: object
                .get("y")
                .and_then(Value::as_f64)
                .ok_or("bounty.y is missing")?,
        },
        points: object.get("points").and_then(Value::as_f64).unwrap_or(0.0),
        radius: object.get("radius").and_then(Value::as_f64).unwrap_or(0.0),
    })
}

fn parse_body(value: &Value) -> Result<MovingBody, String> {
    let object = value.as_object().ok_or("enemy must be an object")?;
    Ok(MovingBody {
        position: Vec2 {
            x: object
                .get("x")
                .and_then(Value::as_f64)
                .ok_or("enemy.x is missing")?,
            y: object
                .get("y")
                .and_then(Value::as_f64)
                .ok_or("enemy.y is missing")?,
        },
        velocity: vector(object.get("velocity").ok_or("enemy.velocity is missing")?)?,
        alive: object.get("status").and_then(Value::as_str) == Some("alive")
            && object
                .get("health")
                .and_then(Value::as_f64)
                .unwrap_or(100.0)
                > 0.0,
    })
}

#[cfg(test)]
mod tests {
    use super::Desert;
    use serde_json::json;

    #[test]
    fn parses_reference_desert_field_names() {
        let value = json!({
            "errors": [], "mapSize": {"x": 9000, "y": 9000}, "maxAccel": 40,
            "maxSpeed": 110, "transportRadius": 5,
            "transports": [{"id":"p_0", "x":1, "y":2, "velocity":{"x":3,"y":4},
                "selfAcceleration":{"x":0,"y":0}, "anomalyAcceleration":{"x":1,"y":-1},
                "status":"alive", "health":100}],
            "anomalies": [], "bounties": [], "enemies": []
        });
        let desert = Desert::parse(value).unwrap();
        assert_eq!(desert.transports[0].id, "p_0");
        assert_eq!(desert.transports[0].anomaly_acceleration.x, 1.0);
        assert_eq!(desert.max_speed, 110.0);
    }
}
