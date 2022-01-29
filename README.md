# fdwalk

A work-in-progress attempt to build a replacement for `walkdir` that uses the `at`-family of POSIX functions to implement [directory traversal without using filepaths](https://unterwaditzer.net/2021/linux-paths.html). A possible application could be [this rust-coreutils issue](https://github.com/uutils/coreutils/issues/2949).

It attempts to be generic over how paths (for display purposes) are allocated, if they're allocated at all, and generally has questionable API design choices.

I lost motivation to work on this momentarily.
