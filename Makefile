PREFIX ?= /usr/local
BINDIR = $(PREFIX)/bin
ICONDIR = $(PREFIX)/share/icons/hicolor/256x256/apps
DESKTOPDIR = $(PREFIX)/share/applications

.PHONY: build install uninstall

build:
	cargo build --release

install: build
	install -Dm755 target/release/kokoro-reader $(DESTDIR)$(BINDIR)/kokoro-reader
	install -Dm644 assets/icon.png $(DESTDIR)$(ICONDIR)/kokoro-reader.png
	install -Dm644 kokoro-reader.desktop $(DESTDIR)$(DESKTOPDIR)/kokoro-reader.desktop

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/kokoro-reader
	rm -f $(DESTDIR)$(ICONDIR)/kokoro-reader.png
	rm -f $(DESTDIR)$(DESKTOPDIR)/kokoro-reader.desktop
