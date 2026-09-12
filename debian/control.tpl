Source: openuu-server
Section: net
Priority: optional
Maintainer: open-trade <info@rustdesk.com>
Build-Depends: debhelper (>= 10), pkg-config
Standards-Version: 4.5.0
Homepage: https://github.com/VocabVictor/openuu-server

Package: openuu-server-hbbs
Architecture: {{ ARCH }}
Depends: systemd ${misc:Depends}
Description: OpenUU server
 Self-host your own OpenUU server, it is free and open source.

Package: openuu-server-hbbr
Architecture: {{ ARCH }}
Depends: systemd ${misc:Depends}
Description: OpenUU server
 Self-host your own OpenUU server, it is free and open source.
 This package contains the OpenUU relay server.

Package: openuu-server-utils
Architecture: {{ ARCH }}
Depends: ${misc:Depends}
Description: OpenUU server
 Self-host your own OpenUU server, it is free and open source.
 This package contains the openuu-utils binary.

Package: openuu-server-account
Architecture: {{ ARCH }}
Depends: systemd ${misc:Depends}
Description: OpenUU account service
 Account authentication and relay authorization for OpenUU.
