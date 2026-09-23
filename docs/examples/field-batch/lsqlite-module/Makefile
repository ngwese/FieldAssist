# Build lsqlite3.so for field-batch (Lua 5.4 C ABI, amalgamation).
#
# Requires: curl, unzip, a C compiler. Lua 5.4 headers are auto-detected or
# fetched from GitHub when LUA_INCDIR is unset.
#
#   make
#   field-batch index-audio-files.lua /path/to/media ./out.sqlite

UNAME_S := $(shell uname -s)

LSQLITE_VERSION := 0.9.6
LSQLITE_ZIP := lsqlite3_v096.zip
LSQLITE_URL := https://github.com/bhhaskin/lsqlite3complete/releases/download/0.9.6-1/$(LSQLITE_ZIP)
LSQLITE_DIR := lsqlite3_v096
LSQLITE_STAMP := .lsqlite3-src

LUA_VERSION := 5.4.7
LUA_TARBALL := lua-$(LUA_VERSION).tar.gz
# lua.org FTP often 403s from automated clients; GitHub tree has headers at top.
LUA_URL := https://github.com/lua/lua/archive/refs/tags/v$(LUA_VERSION).tar.gz
LUA_SRC_DIR := lua-$(LUA_VERSION)
LUA_STAMP := .lua-src

# Prefer an existing Lua 5.4 include tree; otherwise use fetched sources.
ifeq ($(origin LUA_INCDIR), undefined)
  LUA_INCDIR := $(shell \
    (pkg-config --variable=includedir lua5.4 2>/dev/null) \
    || (pkg-config --variable=includedir lua-5.4 2>/dev/null) \
    || (test -d /opt/homebrew/opt/lua@5.4/include/lua5.4 && echo /opt/homebrew/opt/lua@5.4/include/lua5.4) \
    || (test -d /usr/local/opt/lua@5.4/include/lua5.4 && echo /usr/local/opt/lua@5.4/include/lua5.4) \
    || (test -d /usr/include/lua5.4 && echo /usr/include/lua5.4) \
    || true)
endif

ifeq ($(strip $(LUA_INCDIR)),)
  LUA_INCDIR := $(LUA_SRC_DIR)
  NEED_LUA_SRC := 1
endif

CC ?= cc
CFLAGS ?= -O2 -fPIC -I$(LUA_INCDIR) -DLSQLITE_VERSION=\"$(LSQLITE_VERSION)\"
# Do not link liblua — resolve lua_* from the field-batch process (-rdynamic).

ifeq ($(UNAME_S),Darwin)
  SHARED_FLAGS := -bundle -undefined dynamic_lookup
else
  SHARED_FLAGS := -shared
endif

.PHONY: all clean

all: lsqlite3.so

$(LSQLITE_ZIP):
	curl -fsSL -o $@ "$(LSQLITE_URL)"

$(LSQLITE_STAMP): $(LSQLITE_ZIP)
	rm -rf "$(LSQLITE_DIR)"
	unzip -q -o "$(LSQLITE_ZIP)"
	@test -f "$(LSQLITE_DIR)/lsqlite3.c" && test -f "$(LSQLITE_DIR)/sqlite3.c"
	@touch $@

$(LUA_TARBALL):
	curl -fsSL -o $@ "$(LUA_URL)"

$(LUA_STAMP): $(LUA_TARBALL)
	rm -rf "$(LUA_SRC_DIR)"
	tar -xzf "$(LUA_TARBALL)"
	@test -f "$(LUA_SRC_DIR)/lua.h"
	@touch $@

ifeq ($(NEED_LUA_SRC),1)
lsqlite3.so: $(LSQLITE_STAMP) $(LUA_STAMP)
else
lsqlite3.so: $(LSQLITE_STAMP)
endif
	$(CC) $(CFLAGS) $(SHARED_FLAGS) -o $@ \
		$(LSQLITE_DIR)/lsqlite3.c $(LSQLITE_DIR)/sqlite3.c
	@echo "built $@ (LUA_INCDIR=$(LUA_INCDIR))"

clean:
	rm -f lsqlite3.so $(LSQLITE_ZIP) $(LUA_TARBALL) $(LSQLITE_STAMP) $(LUA_STAMP)
	rm -rf $(LSQLITE_DIR) $(LUA_SRC_DIR)
