PREFIX ?= /usr/local
BINDIR = $(PREFIX)/bin
LIBDIR = $(PREFIX)/lib
ICONDIR = $(PREFIX)/share/icons/hicolor/256x256/apps
DESKTOPDIR = $(PREFIX)/share/applications

.PHONY: build install uninstall

build:
	cargo build --release

install:
	install -Dm755 kokoro-reader $(DESTDIR)$(BINDIR)/kokoro-reader
	install -Dm755 libpdfium.so $(DESTDIR)$(LIBDIR)/libpdfium.so
	install -Dm644 kokoro-reader.png $(DESTDIR)$(ICONDIR)/kokoro-reader.png
	install -Dm644 kokoro-reader.desktop $(DESTDIR)$(DESKTOPDIR)/kokoro-reader.desktop

uninstall:
	rm -f $(DESTDIR)$(BINDIR)/kokoro-reader
	rm -f $(DESTDIR)$(LIBDIR)/libpdfium.so
	rm -f $(DESTDIR)$(ICONDIR)/kokoro-reader.png
	rm -f $(DESTDIR)$(DESKTOPDIR)/kokoro-reader.desktop
