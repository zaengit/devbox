Name:           devbox
Version:        0.1.0
Release:        1%{?dist}
Summary:        Native development environment manager
License:        MIT
URL:            https://github.com/devbox-project/devbox
Source0:        %{name}-%{version}.tar.gz
BuildRequires:  cargo
BuildRequires:  cmake
BuildRequires:  pkgconfig
BuildRequires:  alsa-lib-devel
BuildRequires:  libxkbcommon-devel
BuildRequires:  wayland-devel
BuildRequires:  libX11-devel
BuildRequires:  libXcursor-devel
BuildRequires:  libXi-devel
BuildRequires:  libXrandr-devel
BuildRequires:  vulkan-loader-devel
Requires:       alsa-lib
Requires:       libxkbcommon
Requires:       wayland
Requires:       libX11
Requires:       libXcursor
Requires:       libXi
Requires:       libXrandr
Requires:       vulkan-loader

%description
Devbox discovers installed host runtimes, runs projects as the current user,
and manages explicitly configured user-owned service processes.

%prep
%autosetup -n devbox-%{version}

%build
cargo build --locked --release -p devbox-app

%install
install -Dpm0755 target/release/devbox-app %{buildroot}%{_bindir}/devbox
install -Dpm0644 packaging/devbox.desktop %{buildroot}%{_datadir}/applications/devbox.desktop
install -Dpm0644 assets/icons/boxes.svg %{buildroot}%{_datadir}/icons/hicolor/scalable/apps/devbox.svg
install -Dpm0644 LICENSE %{buildroot}%{_licensedir}/%{name}/LICENSE
install -Dpm0644 assets/icons/LICENSE-lucide.txt %{buildroot}%{_licensedir}/%{name}/LICENSE-lucide.txt
install -Dpm0644 README.md %{buildroot}%{_docdir}/%{name}/README.md

%files
%{_bindir}/devbox
%{_datadir}/applications/devbox.desktop
%{_datadir}/icons/hicolor/scalable/apps/devbox.svg
%license %{_licensedir}/%{name}/LICENSE
%license %{_licensedir}/%{name}/LICENSE-lucide.txt
%doc %{_docdir}/%{name}/README.md
