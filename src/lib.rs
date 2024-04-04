//! Rust implementation of [the UAPI Configuration Files Specification.](https://uapi-group.org/specifications/specs/configuration_files_specification/)
//!
//! The tl;dr of the spec is:
//!
//! - The search acts on a list of search directories. The directories are considered in the order that they appear in the list.
//!
//! - In each search directory, the algorithm looks for a config file with the requested filename.
//!
//! - In each search directory, a directory with a name that is the requested name followed by ".d" is considered to be a dropin directory.
//!   Any files in such dropin directories are also considered, in lexicographic order of filename.
//!
//! - If a file with the requested name is present in more than one search directory, the file in the earlier search directory is ignored.
//!   If a dropin file with the same name is present in more than one dropin directory, the file in the earlier dropin directory is ignored.
//!
//! - The config file, if found, is yielded, followed by any dropins that were found. Settings in files yielded later override settings in files yielded earlier.
//!
//! - If no filename is requested, then the algorithm only looks for a directory with a name that is the project name followed by ".d" under all search directories,
//!   and treats those directories as dropin directories.
//!
//! ```rust
//! let files /* : impl Iterator<Item = (PathBuf, File)> */ =
//!     uapi_config::SearchDirectories::modern_system()
//!     .with_project("foobar")
//!     .find_files(".conf")
//!     .unwrap();
//! ```

use std::{
	borrow::Cow,
	collections::BTreeMap,
	ffi::{OsStr, OsString},
	fs::{self, File},
	io,
	ops::Deref,
	os::unix::ffi::{OsStrExt as _, OsStringExt as _},
	path::{Component, Path, PathBuf},
};

/// A list of search directories that the config files will be searched under.
#[derive(Clone, Debug)]
pub struct SearchDirectories<'a> {
	inner: Vec<Cow<'a, Path>>,
}

impl<'a> SearchDirectories<'a> {
	/// Start with an empty list of search directories.
	pub const fn empty() -> Self {
		Self {
			inner: vec![],
		}
	}

	/// Start with the default search directory roots for a system application on a classic Linux distribution.
	///
	/// The OS vendor ships configuration in `/usr/lib`, ephemeral configuration is defined in `/var/run`,
	/// and the sysadmin places overrides in `/etc`.
	pub fn classic_system() -> Self {
		Self {
			inner: vec![
				Path::new("/usr/lib").into(),
				Path::new("/var/run").into(),
				Path::new("/etc").into(),
			],
		}
	}

	/// Start with the default search directory roots for a system application on a modern Linux distribution.
	///
	/// The OS vendor ships configuration in `/usr/etc`, ephemeral configuration is defined in `/run`,
	/// and the sysadmin places overrides in `/etc`.
	pub fn modern_system() -> Self {
		Self {
			inner: vec![
				Path::new("/usr/etc").into(),
				Path::new("/run").into(),
				Path::new("/etc").into(),
			],
		}
	}

	/// Append the directory for local user config overrides, `$XDG_CONFIG_HOME`.
	///
	/// If the `dirs` crate feature is enabled, then `dirs::config_dir()` is used for the implementation of `$XDG_CONFIG_HOME`.
	/// else a custom implementation is used.
	#[must_use]
	pub fn with_user_directory(mut self) -> Self {
		let user_config_dir;
		#[cfg(feature = "dirs")]
		{
			user_config_dir = dirs::config_dir();
		}
		#[cfg(not(feature = "dirs"))]
		match std::env::var_os("XDG_CONFIG_HOME") {
			Some(value) if !value.is_empty() => user_config_dir = Some(value.into()),

			_ => match std::env::var_os("HOME") {
				Some(value) if !value.is_empty() => {
					let mut value: PathBuf = value.into();
					value.push(".config");
					user_config_dir = Some(value);
				},

				_ => {
					user_config_dir = None;
				},
			},
		}

		if let Some(user_config_dir) = user_config_dir {
			// If the value fails validation, ignore it.
			_ = self.push(user_config_dir.into());
		}

		self
	}

	/// Prepend the specified path to all search directories.
	///
	/// # Errors
	///
	/// Returns `Err(InvalidPathError)` if `root` does not start with a [`Component::RootDir`] or if it contains [`Component::ParentDir`].
	pub fn chroot(mut self, root: &Path) -> Result<Self, InvalidPathError> {
		validate_path(root)?;

		for dir in &mut self.inner {
			let mut new_dir = root.to_owned();
			for component in dir.components() {
				match component {
					Component::Prefix(_) => unreachable!("this variant is Windows-only"),
					Component::RootDir |
					Component::CurDir => (),
					Component::ParentDir => unreachable!("all paths in self.inner went through validate_path or were hard-coded to be valid"),
					Component::Normal(component) => {
						new_dir.push(component);
					},
				}
			}
			*dir = new_dir.into();
		}

		Ok(self)
	}

	/// Appends a search directory to the end of the list.
	/// Files found in this directory will override files found in earlier directories.
	///
	/// # Errors
	///
	/// Returns `Err(InvalidPathError)` if `path` does not start with a [`Component::RootDir`] or if it contains [`Component::ParentDir`].
	pub fn push(&mut self, path: Cow<'a, Path>) -> Result<(), InvalidPathError> {
		validate_path(&path)?;

		self.inner.push(path);

		Ok(())
	}

	/// Search for configuration files for the given project name.
	///
	/// The project name is usually the name of your application.
	pub fn with_project<TProject>(
		self,
		project: TProject,
	) -> SearchDirectoriesForProject<'a, TProject>
	{
		SearchDirectoriesForProject {
			inner: self.inner,
			project,
		}
	}

	/// Search for configuration files with the given config file name.
	pub fn with_file_name<TFileName>(
		self,
		file_name: TFileName,
	) -> SearchDirectoriesForFileName<'a, TFileName>
	{
		SearchDirectoriesForFileName {
			inner: self.inner,
			file_name,
		}
	}
}

impl Default for SearchDirectories<'_> {
	fn default() -> Self {
		Self::empty()
	}
}

impl<'a> FromIterator<Cow<'a, Path>> for SearchDirectories<'a> {
	fn from_iter<T>(iter: T) -> Self where T: IntoIterator<Item = Cow<'a, Path>> {
		Self {
			inner: FromIterator::from_iter(iter),
		}
	}
}

/// Error returned when a path does not start with [`Component::RootDir`] or when it contains [`Component::ParentDir`].
#[derive(Debug)]
pub struct InvalidPathError;

impl std::fmt::Display for InvalidPathError {
	fn fmt(&self, f: &mut std::fmt::Formatter<'_>) -> std::fmt::Result {
		f.write_str("path contains Component::ParentDir")
	}
}

impl std::error::Error for InvalidPathError {}

/// A list of search directories that the config files will be searched under, scoped to a particular project.
#[derive(Clone, Debug)]
pub struct SearchDirectoriesForProject<'a, TProject> {
	inner: Vec<Cow<'a, Path>>,
	project: TProject,
}

impl<'a, TProject> SearchDirectoriesForProject<'a, TProject> {
	/// Search for configuration files of this project with the given config file name.
	pub fn with_file_name<TFileName>(
		self,
		file_name: TFileName,
	) -> SearchDirectoriesForProjectAndFileName<'a, TProject, TFileName>
	{
		SearchDirectoriesForProjectAndFileName {
			inner: self.inner,
			project: self.project,
			file_name,
		}
	}

	/// Returns an [`Iterator`] of `(`[`PathBuf`]`, `[`File`]`)`s for all the files found in the specified search directories.
	/// The name `format!("{project}.d")` is appended to each search directory, then those directories are searched as if they are
	/// dropin directories. Only dropin files whose name ends with `dropin_suffix` will be considered.
	/// Note that if you intend to use a file extension as a suffix, then `dropin_suffix` must include the `.`, such as `".conf"`.
	///
	/// You will likely want to parse each file returned by this function according to whatever format they're supposed to contain
	/// and merge them into a unified config object, with settings from later files overriding settings from earlier files.
	/// This function does not guarantee that the files are well-formed, only that they exist and could be opened for reading.
	///
	/// # Errors
	///
	/// Any errors from reading non-existing directories and non-existing files are ignored.
	/// Apart from that, any I/O errors from walking the directories and from opening the files found within are propagated.
	///
	/// # Examples
	///
	/// ## Get all config files for the system service `foobar`
	///
	/// ... taking into account OS vendor configs, ephemeral overrides and sysadmin overrides.
	///
	/// ```rust
	/// let files =
	///     uapi_config::SearchDirectories::modern_system()
	///     .with_project("foobar")
	///     .find_files(".conf")
	///     .unwrap();
	/// ```
	///
	/// This will locate all dropins `/usr/etc/foobar.d/*.conf`, `/run/foobar.d/*.conf`, `/etc/foobar.d/*.conf` in lexicographical order.
	///
	/// ## Get all config files for the application `foobar`
	///
	/// ... taking into account OS vendor configs, ephemeral overrides, sysadmin overrides and local user overrides.
	///
	/// ```rust
	/// let files =
	///     uapi_config::SearchDirectories::modern_system()
	///     .with_user_directory()
	///     .with_project("foobar")
	///     .find_files(".conf")
	///     .unwrap();
	/// ```
	///
	/// This will locate all dropins `/usr/etc/foobar.d/*.conf`, `/run/foobar.d/*.conf`, `/etc/foobar.d/*.conf`, `$XDG_CONFIG_HOME/foobar.d/*.conf`
	/// in lexicographical order.
	///
	#[cfg_attr(feature = "dirs", doc = r#"## Get all config files for the application `foobar`"#)]
	#[cfg_attr(feature = "dirs", doc = r#""#)]
	#[cfg_attr(feature = "dirs", doc = r#"... with custom paths for the OS vendor configs, sysadmin overrides and local user overrides."#)]
	#[cfg_attr(feature = "dirs", doc = r#""#)]
	#[cfg_attr(feature = "dirs", doc = r#"```rust"#)]
	#[cfg_attr(feature = "dirs", doc = r#"// OS and sysadmin configs"#)]
	#[cfg_attr(feature = "dirs", doc = r#"let mut search_directories: uapi_config::SearchDirectories = ["#)]
	#[cfg_attr(feature = "dirs", doc = r#"    std::path::Path::new("/usr/share").into(),"#)]
	#[cfg_attr(feature = "dirs", doc = r#"    std::path::Path::new("/etc").into(),"#)]
	#[cfg_attr(feature = "dirs", doc = r#"].into_iter().collect();"#)]
	#[cfg_attr(feature = "dirs", doc = r#""#)]
	#[cfg_attr(feature = "dirs", doc = r#"// Local user configs under `${XDG_CONFIG_HOME:-$HOME/.config}`"#)]
	#[cfg_attr(feature = "dirs", doc = r#"if let Some(user_config_dir) = dirs::config_dir() {"#)]
	#[cfg_attr(feature = "dirs", doc = r#"    search_directories.push(user_config_dir.into());"#)]
	#[cfg_attr(feature = "dirs", doc = r#"}"#)]
	#[cfg_attr(feature = "dirs", doc = r#""#)]
	#[cfg_attr(feature = "dirs", doc = r#"let files ="#)]
	#[cfg_attr(feature = "dirs", doc = r#"    search_directories"#)]
	#[cfg_attr(feature = "dirs", doc = r#"    .with_project("foobar")"#)]
	#[cfg_attr(feature = "dirs", doc = r#"    .find_files(".conf")"#)]
	#[cfg_attr(feature = "dirs", doc = r#"    .unwrap();"#)]
	#[cfg_attr(feature = "dirs", doc = r#"```"#)]
	#[cfg_attr(feature = "dirs", doc = r#""#)]
	#[cfg_attr(feature = "dirs", doc = r#"This will locate `/usr/share/foobar.d/*.conf`, `/etc/foobar.d/*.conf`, `$XDG_CONFIG_HOME/foobar.d/*.conf` in that order and return the last one."#)]
	pub fn find_files<TDropinSuffix>(
		self,
		dropin_suffix: TDropinSuffix,
	) -> io::Result<Files>
	where
		TProject: AsRef<OsStr>,
		TDropinSuffix: AsRef<OsStr>,
	{
		let project = self.project.as_ref().as_bytes();

		let dropins = find_dropins(dropin_suffix.as_ref(), self.inner.into_iter().map(|path| {
			let mut path_bytes = path.into_owned().into_os_string().into_vec();
			path_bytes.push(b'/');
			path_bytes.extend_from_slice(project);
			path_bytes.extend_from_slice(b".d");
			PathBuf::from(OsString::from_vec(path_bytes))
		}))?;

		Ok(Files {
			inner: None.into_iter().chain(dropins),
		})
	}
}

/// A list of search directories that the config files will be searched under, scoped to a particular config file name.
#[derive(Clone, Debug)]
pub struct SearchDirectoriesForFileName<'a, TFileName> {
	inner: Vec<Cow<'a, Path>>,
	file_name: TFileName,
}

impl<'a, TFileName> SearchDirectoriesForFileName<'a, TFileName> {
	/// Search for configuration files for the given project name and with this config file name.
	///
	/// The project name is usually the name of your application.
	pub fn with_project<TProject>(
		self,
		project: TProject,
	) -> SearchDirectoriesForProjectAndFileName<'a, TProject, TFileName>
	{
		SearchDirectoriesForProjectAndFileName {
			inner: self.inner,
			project,
			file_name: self.file_name,
		}
	}

	/// Returns an [`Iterator`] of `(`[`PathBuf`]`, `[`File`]`)`s for all the files found in the specified search directories.
	/// Only files named `file_name` under the search directories will be considered.
	///
	/// If `dropin_suffix` is provided, then directories named `format!("{file_name}.d")` under the search directories are treated as dropin directories.
	/// Only dropin files whose name ends with `dropin_suffix` will be considered. Note that if you intend to use a file extension as a suffix,
	/// then `dropin_suffix` must include the `.`, such as `".conf"`.
	///
	/// You will likely want to parse each file returned by this function according to whatever format they're supposed to contain
	/// and merge them into a unified config object, with settings from later files overriding settings from earlier files.
	/// This function does not guarantee that the files are well-formed, only that they exist and could be opened for reading.
	///
	/// # Errors
	///
	/// Any errors from reading non-existing directories and non-existing files are ignored.
	/// Apart from that, any I/O errors from walking the directories and from opening the files found within are propagated.
	///
	/// # Examples
	///
	/// ## Get all config files for the system service `foobar`
	///
	/// ... taking into account OS vendor configs, ephemeral overrides and sysadmin overrides.
	///
	/// ```rust
	/// let files =
	///     uapi_config::SearchDirectories::modern_system()
	///     .with_file_name("foobar.conf")
	///     .find_files(Some(".conf"))
	///     .unwrap();
	/// ```
	///
	/// This will locate `/usr/etc/foobar.conf` `/run/foobar.conf`, `/etc/foobar.conf` in that order and return the last one,
	/// then all dropins `/usr/etc/foobar.d/*.conf`, `/run/foobar.d/*.conf`, `/etc/foobar.d/*.conf` in lexicographical order.
	///
	/// ## Get the config files for the "foo.service" systemd system unit like systemd would do
	///
	/// ```rust
	/// let search_directories: uapi_config::SearchDirectories =
	///     // From `man systemd.unit`
	///     [
	///         "/run/systemd/generator.late",
	///         "/usr/lib/systemd/system",
	///         "/usr/local/lib/systemd/system",
	///         "/run/systemd/generator",
	///         "/run/systemd/system",
	///         "/etc/systemd/system",
	///         "/run/systemd/generator.early",
	///         "/run/systemd/transient",
	///         "/run/systemd/system.control",
	///         "/etc/systemd/system.control",
	///     ].into_iter()
	///     .map(|path| std::path::Path::new(path).into())
	///     .collect();
	/// let files =
	///     search_directories
	///     .with_file_name("foo.service")
	///     .find_files(Some(".conf"))
	///     .unwrap();
	/// ```
	///
	/// This will locate `/run/systemd/generator.late/foobar.service` `/usr/lib/systemd/system/foo.service`, ... in that order and return the last one,
	/// then all dropins `/run/systemd/generator.late/foo.service.d/*.conf`, `/usr/lib/systemd/system/foo.service.d/*.conf`, ... in lexicographical order.
	pub fn find_files<TDropinSuffix>(
		self,
		dropin_suffix: Option<TDropinSuffix>,
	) -> io::Result<Files>
	where
		TFileName: AsRef<OsStr>,
		TDropinSuffix: AsRef<OsStr>,
	{
		let file_name = self.file_name.as_ref();

		let main_file = find_main_file(file_name, self.inner.iter().map(Deref::deref))?;

		let dropins =
			if let Some(dropin_suffix) = dropin_suffix {
				find_dropins(dropin_suffix.as_ref(), self.inner.into_iter().map(|path| {
					let mut path_bytes = path.into_owned().into_os_string().into_vec();
					path_bytes.push(b'/');
					path_bytes.extend_from_slice(file_name.as_bytes());
					path_bytes.extend_from_slice(b".d");
					PathBuf::from(OsString::from_vec(path_bytes))
				}))?
			}
			else {
				Default::default()
			};

		Ok(Files {
			inner: main_file.into_iter().chain(dropins),
		})
	}
}

/// A list of search directories that the config files will be searched under, scoped to a particular project and config file name.
#[derive(Clone, Debug)]
pub struct SearchDirectoriesForProjectAndFileName<'a, TProject, TFileName> {
	inner: Vec<Cow<'a, Path>>,
	project: TProject,
	file_name: TFileName,
}

impl<'a, TProject, TFileName> SearchDirectoriesForProjectAndFileName<'a, TProject, TFileName> {
	/// Returns an [`Iterator`] of `(`[`PathBuf`]`, `[`File`]`)`s for all the files found in the specified search directories.
	/// The project name is appended to each search directory, then those directories are searched for files named `file_name`.
	///
	/// If `dropin_suffix` is provided, then directories named `format!("{file_name}.d")` under the search directories are treated as dropin directories.
	/// Only dropin files whose name ends with `dropin_suffix` will be considered. Note that if you intend to use a file extension as a suffix,
	/// then `dropin_suffix` must include the `.`, such as `".conf"`.
	///
	/// You will likely want to parse each file returned by this function according to whatever format they're supposed to contain
	/// and merge them into a unified config object, with settings from later files overriding settings from earlier files.
	/// This function does not guarantee that the files are well-formed, only that they exist and could be opened for reading.
	///
	/// # Errors
	///
	/// Any errors from reading non-existing directories and non-existing files are ignored.
	/// Apart from that, any I/O errors from walking the directories and from opening the files found within are propagated.
	pub fn find_files<TDropinSuffix>(
		self,
		dropin_suffix: Option<TDropinSuffix>,
	) -> io::Result<Files>
	where
		TProject: AsRef<OsStr>,
		TFileName: AsRef<OsStr>,
		TDropinSuffix: AsRef<OsStr>,
	{
		let project = self.project.as_ref();

		let file_name = self.file_name.as_ref();

		let main_file = find_main_file(file_name, self.inner.iter().map(|path| path.join(project)))?;

		let dropins =
			if let Some(dropin_suffix) = dropin_suffix {
				find_dropins(dropin_suffix.as_ref(), self.inner.into_iter().map(|path| {
					let mut path_bytes = path.into_owned().into_os_string().into_vec();
					path_bytes.push(b'/');
					path_bytes.extend_from_slice(project.as_bytes());
					path_bytes.push(b'/');
					path_bytes.extend_from_slice(file_name.as_bytes());
					path_bytes.extend_from_slice(b".d");
					PathBuf::from(OsString::from_vec(path_bytes))
				}))?
			}
			else {
				Default::default()
			};

		Ok(Files {
			inner: main_file.into_iter().chain(dropins),
		})
	}
}

fn validate_path(path: &Path) -> Result<(), InvalidPathError> {
	let mut components = path.components();

	if components.next() != Some(Component::RootDir) {
		return Err(InvalidPathError);
	}

	if components.any(|component| matches!(component, Component::ParentDir)) {
		return Err(InvalidPathError);
	}

	Ok(())
}

fn find_main_file<I>(
	file_name: &OsStr,
	search_directories: I,
) -> io::Result<Option<(PathBuf, File)>>
where
	I: DoubleEndedIterator,
	I::Item: Deref<Target = Path>,
{
	for search_directory in search_directories.rev() {
		let path = search_directory.join(file_name);
		let file = match File::open(&path) {
			Ok(file) => file,
			Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
			Err(err) => return Err(err),
		};

		if !file.metadata()?.file_type().is_file() {
			continue;
		}

		return Ok(Some((path, file)));
	}

	Ok(None)
}

fn find_dropins<I>(
	suffix: &OsStr,
	search_directories: I,
) -> io::Result<std::collections::btree_map::IntoValues<Vec<u8>, (PathBuf, File)>>
where
	I: DoubleEndedIterator,
	I::Item: Deref<Target = Path>,
{
	let mut result: BTreeMap<_, _> = Default::default();

	for search_directory in search_directories.rev() {
		let entries = match fs::read_dir(&*search_directory) {
			Ok(entries) => entries,
			Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
			Err(err) => return Err(err),
		};
		for entry in entries {
			let entry = entry?;

			let file_name = entry.file_name();
			if !file_name.as_bytes().ends_with(suffix.as_bytes()) {
				continue;
			}

			if result.contains_key(file_name.as_bytes()) {
				continue;
			}

			let path = search_directory.join(&file_name);
			let file = match File::open(&path) {
				Ok(file) => file,
				Err(err) if err.kind() == io::ErrorKind::NotFound => continue,
				Err(err) => return Err(err),
			};

			if !file.metadata()?.file_type().is_file() {
				continue;
			}

			result.insert(file_name.into_vec(), (path, file));
		}
	}

	Ok(result.into_values())
}

/// The iterator of files returned by [`SearchDirectoriesForProject::find_files`],
/// [`SearchDirectoriesForFileName::find_files`] and [`SearchDirectoriesForProjectAndFileName::find_files`].
#[derive(Debug)]
#[repr(transparent)]
pub struct Files {
	inner: FilesInner,
}

type FilesInner =
	std::iter::Chain<
		std::option::IntoIter<(PathBuf, File)>,
		std::collections::btree_map::IntoValues<Vec<u8>, (PathBuf, File)>,
	>;

impl Iterator for Files {
	type Item = (PathBuf, File);

	fn next(&mut self) -> Option<Self::Item> {
		self.inner.next()
	}
}

impl DoubleEndedIterator for Files {
	fn next_back(&mut self) -> Option<Self::Item> {
		self.inner.next_back()
	}
}

const _STATIC_ASSERT_FILES_INNER_IS_FUSED_ITERATOR: () = {
	const fn is_fused_iterator<T>() where T: std::iter::FusedIterator {}
	is_fused_iterator::<FilesInner>();
};
impl std::iter::FusedIterator for Files {}

#[cfg(test)]
mod tests {
	use std::path::{Path, PathBuf};

	use crate::SearchDirectories;

	#[test]
	fn search_directory_precedence() {
		for include_usr_etc in [false, true] {
			for include_run in [false, true] {
				for include_etc in [false, true] {
					let mut search_directories = vec![];
					if include_usr_etc {
						search_directories.push(concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/usr/etc"));
					}
					if include_run {
						search_directories.push(concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/run"));
					}
					if include_etc {
						search_directories.push(concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/etc"));
					}
					let search_directories: SearchDirectories<'_> =
						search_directories
						.into_iter()
						.map(|path| Path::new(path).into())
						.collect();
					let files: Vec<_> =
						search_directories
						.with_project("foo")
						.with_file_name("a.conf")
						.find_files(Some(".conf"))
						.unwrap()
						.map(|(path, _)| path)
						.collect();
					if include_etc {
						assert_eq!(files, [
							concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/etc/foo/a.conf"),
							concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/etc/foo/a.conf.d/b.conf"),
						].into_iter().map(Into::into).collect::<Vec<PathBuf>>());
					}
					else if include_run {
						assert_eq!(files, [
							concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/run/foo/a.conf"),
							concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/run/foo/a.conf.d/b.conf"),
						].into_iter().map(Into::into).collect::<Vec<PathBuf>>());
					}
					else if include_usr_etc {
						assert_eq!(files, [
							concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/usr/etc/foo/a.conf"),
							concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/search_directory_precedence/usr/etc/foo/a.conf.d/b.conf"),
						].into_iter().map(Into::into).collect::<Vec<PathBuf>>());
					}
					else {
						assert_eq!(files, Vec::<PathBuf>::new());
					}
				}
			}
		}
	}

	#[test]
	fn only_project() {
		let files: Vec<_> =
			SearchDirectories::modern_system()
			.chroot(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_project")))
			.unwrap()
			.with_project("foo")
			.find_files(".conf")
			.unwrap()
			.map(|(path, _)| path)
			.collect();
		assert_eq!(files, [
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_project/etc/foo.d/a.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_project/usr/etc/foo.d/b.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_project/run/foo.d/c.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_project/etc/foo.d/d.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_project/run/foo.d/e.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_project/usr/etc/foo.d/f.conf"),
		].into_iter().map(Into::into).collect::<Vec<PathBuf>>());
	}

	#[test]
	fn only_file_name() {
		let files: Vec<_> =
			SearchDirectories::modern_system()
			.chroot(Path::new(concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name")))
			.unwrap()
			.with_file_name("foo.service")
			.find_files(Some(".conf"))
			.unwrap()
			.map(|(path, _)| path)
			.collect();
		assert_eq!(files, [
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name/etc/foo.service"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name/etc/foo.service.d/a.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name/usr/etc/foo.service.d/b.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name/run/foo.service.d/c.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name/etc/foo.service.d/d.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name/run/foo.service.d/e.conf"),
			concat!(env!("CARGO_MANIFEST_DIR"), "/test-files/only_file_name/usr/etc/foo.service.d/f.conf"),
		].into_iter().map(Into::into).collect::<Vec<PathBuf>>());
	}
}
