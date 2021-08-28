#!/bin/make

build:
	cargo build --release

install:
	install -st /usr/local/bin/ target/release/cwm
	install -st /usr/local/bin/ target/release/cwm-client
	install -t /usr/share/xsessions/ cwm.desktop