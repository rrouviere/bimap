pub const ICMP_TYPE_ECHO_REPLY: u8 = 0;

pub const ALL_ICMP_TYPES: &[(u8, &str)] = &[
    (0, "echo-reply"),
    (3, "dest-unreachable"),
    (4, "source-quench"),
    (5, "redirect"),
    (8, "echo-request"),
    (9, "router-advertisement"),
    (10, "router-solicitation"),
    (11, "time-exceeded"),
    (12, "param-problem"),
    (13, "timestamp"),
    (14, "timestamp-reply"),
    (15, "info-request"),
    (16, "info-reply"),
    (17, "address-mask"),
    (18, "address-mask-reply"),
    (30, "traceroute"),
    (31, "datagram-conversion"),
    (33, "ipv6-where-are-you"),
    (34, "ipv6-i-am-here"),
    (35, "mobile-registration"),
    (37, "domain-name-request"),
    (38, "domain-name-reply"),
    (40, "photuris"),
    (42, "extended-echo"),
];

pub fn type_name(icmp_type: u8) -> &'static str {
    for &(t, name) in ALL_ICMP_TYPES {
        if t == icmp_type {
            return name;
        }
    }
    "unknown"
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn all_types_have_names() {
        for &(t, name) in ALL_ICMP_TYPES {
            assert!(!name.is_empty(), "type {t} has no name");
        }
    }

    #[test]
    fn type_name_known_types() {
        assert_eq!(type_name(8), "echo-request");
        assert_eq!(type_name(0), "echo-reply");
        assert_eq!(type_name(13), "timestamp");
    }

    #[test]
    fn type_name_unknown() {
        assert_eq!(type_name(99), "unknown");
    }
}
