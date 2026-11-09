// This Source Code Form is subject to the terms of the Mozilla Public License, v. 2.0. If a copy of the MPL was not distributed with this file, You can obtain one at https://mozilla.org/MPL/2.0/.

// The engine, from crates/reversal-ios. The app links it and passes it the page's requests.

#include <stdbool.h>
#include <stddef.h>
#include <stdint.h>

/// An answer from the engine, to give back to reversal_response_free.
typedef struct {
  uint16_t status;
  /// One "Name: value" per line.
  char *headers;
  uint8_t *body;
  size_t body_len;
} ReversalResponse;

/// Opens `folder` in place of the one that was open, as reversal_engine_close would close it.
/// If that fails, no folder is open.
bool reversal_engine_open(const char *folder);
/// Closes the folder. Frames being saved finish; the rest of a save doesn't start.
void reversal_engine_close(void);
/// Lets go of the frames the engine keeps ready to show, when the system is short of memory.
void reversal_engine_trim(void);
/// Answers one request from the page: `method` is GET or POST, `url` the path and query, and
/// `headers` one "Name: value" per line. Waits for a request that's being answered. Never null.
ReversalResponse *reversal_engine_request(
    const char *method, const char *url, const char *headers, const uint8_t *body, size_t body_len);
void reversal_response_free(ReversalResponse *response);
/// Whether the engine would list a file of this name as a frame.
bool reversal_is_scan(const char *name);
