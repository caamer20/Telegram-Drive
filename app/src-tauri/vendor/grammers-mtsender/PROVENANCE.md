# Pinned Grammers transport resolver compatibility backport

This directory contains `grammers-mtsender` 0.8.1 from the application's existing
Grammers revision `d07f96f6bee22a3a850a1f3067834686a08b396c`. The originating public
repository is https://github.com/Lonami/grammers; maintained upstream moved to
https://codeberg.org/Lonami/grammers. This copy does not adopt upstream's newer
protocol or session versions. Sibling Grammers dependencies retain the same
original Git revision and version requirements.

`provenance.json` records source and vendored SHA-256 hashes.
`compatibility.patch` contains the exact changes to copied upstream files and the
added macOS configuration helper. Both upstream license alternatives are retained
in `LICENSE-APACHE` and `LICENSE-MIT`.

## Runtime changes

- Update only the transport's optional resolver from Hickory 0.25.2 to 0.26.3.
  The resolver's `system-config` and `tokio` default features remain enabled.
  No DNSSEC, encrypted-DNS, or alternate DNS-provider feature is enabled or disabled.
- Adapt fallible resolver construction and lookup errors to `io::Error::other`,
  which is also how the old `ResolveError` conversion represented lookup failures.
  A configuration error returns normally instead of the previous `unwrap` panic.
- Preserve first-address selection with explicit `Ipv4thenIpv6`; 0.26.3 otherwise
  defaults to querying both families with IPv6 ordered first. Literal IPv4 and
  IPv6 proxy addresses still bypass hostname resolution, and SOCKS parsing,
  credentials, negotiation, and final target selection are unchanged.
- Preserve negative-response failover with `trust_negative_responses = false`
  for each system nameserver. New resolver configuration constructors default true.
- Keep a conservative cache limit of 32. Hickory 0.25 counted records; 0.26 counts
  complete responses. These units are not identical, and no equivalence in maximum
  cached bytes is claimed. Each proxy connection creates a fresh resolver.
- Preserve Windows EDNS-disabled default. Unix EDNS still comes from the system
  resolver file, including any explicitly enabled `edns0` option.
- Preserve macOS `/etc/resolv.conf` handling rather than switching this pinned
  transport to Hickory 0.26's SystemConfiguration reader. `src/net/resolv_conf.rs`
  adapts the licensed Hickory 0.25.2 Unix reader from revision
  `527c9f470a418cf6b92da902ea0aaa5749963d59`, preserving strict parsing, system
  domain, search list, nameservers, UDP/TCP on port 53, ndots, timeout, attempts,
  EDNS, missing-server error, and the ignored `--` search sentinel. The additional
  macOS-only `resolv-conf` dependency is activated only with the proxy feature.
  The source attribution and both licenses are in `LICENSE-HICKORY-APACHE` and
  `LICENSE-HICKORY-MIT`.

## Remaining resolver-default comparison

Defaults that remain equivalent: ndots 1, timeout 5 seconds, attempts 2, hosts-file
Auto, unset positive/negative TTL limits, two concurrent upstream requests,
preservation of intermediate records, no TCP-on-error flag, QueryStatistics
server ordering, recursion requested, empty avoided-port set, no OS-selected
UDP-port flag, no case randomization, and no configured trust anchor. System
configuration can replace ndots, timeout, attempts, search/domain and nameservers;
the existing system inputs continue to be used.

New 0.26 controls retain their supported defaults: max_active_requests 32, empty
answer allow/deny lists, standard EDNS payload length, and opportunistic encryption
Off. The old check_names option has no 0.26 counterpart; parsing is provided by
the patched resolver. TLS and DNSSEC controls are inactive under both versions'
default feature sets. The new macOS reader is deliberately not used.

Linux and Windows continue through Hickory's public system configuration reader.
For valid configuration the same nameservers/search/settings are supplied. The
new Linux reader can warn and retain valid parts of a malformed resolver file;
this upstream parser behavior is not represented as byte-for-byte old error
compatibility. macOS retains the old strict parsing behavior in the helper.

## Packaging omissions and platform audit

Upstream `tests/`, the `[dev-dependencies]` section, and the inline `#[cfg(test)]`
module at original `src/errors.rs` lines 315–334 are omitted to follow this
repository's application-boundary E2E policy. Automatic integration test discovery
and doctests are disabled in this dependency manifest. Documentation examples
remain as upstream comments. Validation belongs to the application's native
process/loopback SOCKS journeys and release gates.

The unchanged public upstream `src/configuration.rs` includes one
`cfg(not(target_os = "android"))` guard around desktop locale discovery. It is an
existing public dependency platform exclusion, not private Telegram Drive Android
source or a new Android implementation. No private application Android files,
project/configuration trees, signing material, or instrumentation were copied.

## Sources

- Pinned source: https://github.com/Lonami/grammers/tree/d07f96f6bee22a3a850a1f3067834686a08b396c/grammers-mtsender
- New resolver API: https://docs.rs/hickory-resolver/0.26.3/hickory_resolver/struct.Resolver.html
- New fallible builder: https://docs.rs/hickory-resolver/0.26.3/hickory_resolver/struct.ResolverBuilder.html
- New resolver release: https://github.com/hickory-dns/hickory-dns/releases/tag/v0.26.3
- Old macOS/Unix reader: https://github.com/hickory-dns/hickory-dns/blob/527c9f470a418cf6b92da902ea0aaa5749963d59/crates/resolver/src/system_conf/unix.rs
