/* Revisioned documents that the service saves by compare-and-swap. */
"use strict";
// The service refused these writes, so nothing changed. Any other failure,
// such as a lost connection or a 5xx, leaves the outcome unknown.
function writeFailureKind(error) {
  switch (error?.status) {
    case 409:
      return "conflict";
    case 400:
    case 401:
    case 403:
    case 404:
      return "rejected";
    default:
      return "unknown";
  }
}
