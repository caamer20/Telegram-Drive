// Copyright 2015-2017 Benjamin Fry <benjaminfry@me.com>.
//
// Adapted from hickory-resolver 0.25.2 src/system_conf/unix.rs.
// Licensed under either Apache-2.0 or MIT; see LICENSE-HICKORY-APACHE
// and LICENSE-HICKORY-MIT. See PROVENANCE.md for the exact source revision.
//
// Hickory 0.26 moved macOS configuration to SystemConfiguration. Keep this
// pinned transport's /etc/resolv.conf, search, and EDNS behavior instead.

use std::{io, str::FromStr, time::Duration};

use hickory_resolver::{
    config::{NameServerConfig, ResolverConfig, ResolverOpts},
    proto::rr::Name,
};

pub(super) fn read_system_conf() -> io::Result<(ResolverConfig, ResolverOpts)> {
    let data = std::fs::read_to_string("/etc/resolv.conf")?;
    // Retain the old reader's strict parse failure instead of ignoring invalid
    // parts of the file. No alternate nameserver or public DNS fallback is added.
    let parsed = resolv_conf::Config::parse(&data)
        .map_err(|error| io::Error::other(format!("Error parsing resolv.conf: {error}")))?;
    let domain = parsed
        .get_system_domain()
        .and_then(|domain| Name::from_str(domain.as_str()).ok());
    let name_servers = parsed
        .nameservers
        .iter()
        .map(|ip| {
            let mut server = NameServerConfig::udp_and_tcp(ip.into());
            server.trust_negative_responses = false;
            server
        })
        .collect::<Vec<_>>();
    if name_servers.is_empty() {
        return Err(io::Error::other("no nameservers found in config"));
    }
    let mut search = vec![];
    for search_domain in parsed.get_last_search_or_domain() {
        if search_domain == "--" {
            continue;
        }
        search
            .push(Name::from_str_relaxed(search_domain).map_err(|error| {
                io::Error::other(format!("Error parsing resolv.conf: {error}"))
            })?);
    }
    let mut options = ResolverOpts::default();
    options.ndots = parsed.ndots as usize;
    options.timeout = Duration::from_secs(u64::from(parsed.timeout));
    options.attempts = parsed.attempts as usize;
    options.edns0 = parsed.edns0;
    Ok((
        ResolverConfig::from_parts(domain, search, name_servers),
        options,
    ))
}
