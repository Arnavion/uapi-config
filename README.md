Rust implementation of [the UAPI Configuration Files Specification.](https://uapi-group.org/specifications/specs/configuration_files_specification/)

[crates.io](https://crates.io/crates/uapi-config)

[Documentation](https://docs.rs/uapi-config/)

[Changelog](https://github.com/Arnavion/uapi-config/blob/main/CHANGELOG.md)

MSRV: 1.85.1

# What about [`libeconf`](https://github.com/openSUSE/libeconf) ?

Pros of `libeconf`:

- `libeconf` is used by other projects and thus has more eyeballs on it.

- `libeconf` can be loaded from the distribution instead of needing to be linked / bundled with your application.

- `libeconf` is MIT-licensed. This library is AGPL-3.0-only.

Pros of this library:

- `libeconf`'s API not only locates files in the order specified by the UAPI spec, but also parses them as if they contain simple `key <delimiter> value` lines in `[group]`s, and builds a final merged config itself. Thus it cannot be used with config files that use a different syntax. In the Rust ecosystem specifically, it's common to use more complex formats like TOML.

  This library only locates the files, and leaves it to the caller to parse and merge them.

- This is a pure Rust library with entirely safe code and no mandatory dependencies except libstd. Using `libeconf` requires binding to a C library.

- `libeconf::econf_readConfig` supports OS vendor root + ephemeral root + sysadmin root, where OS vendor root can be customized by the user and the other two are hard-coded. This means it cannot be used with other combinations like OS vendor + ephemeral + sysadmin + local user configs. This library supports a default for OS vendor + ephemeral + sysadmin, as well as a default for OS vendor + ephemeral + sysadmin + local user, as well as an arbitrary list of user-provided directories.

- `libeconf` is MIT-licensed. This library is AGPL-3.0-only.


# License

AGPL-3.0-only

```
uapi-config

https://github.com/Arnavion/uapi-config

Copyright 2024 Arnav Singh

This program is free software: you can redistribute it and/or modify
it under the terms of the GNU Affero General Public License as
published by the Free Software Foundation, version 3 of the
License.

This program is distributed in the hope that it will be useful,
but WITHOUT ANY WARRANTY; without even the implied warranty of
MERCHANTABILITY or FITNESS FOR A PARTICULAR PURPOSE.  See the
GNU Affero General Public License for more details.

You should have received a copy of the GNU Affero General Public License
along with this program.  If not, see <https://www.gnu.org/licenses/>.
```
