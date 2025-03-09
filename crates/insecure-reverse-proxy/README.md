# insecure-reverse-proxy

The reverse proxy code in this utility crate is taken almost entirely from [`hyper-reverse-proxy`](https://github.com/felipenoris/hyper-reverse-proxy). The changes are as follows:

- Updated all dependencies
- Modified so it is more generic
- Implemented a tower service so that it can be used with tower and axum.

As such all credit should go to the authors of `hyper-reverse-proxy` and their license has been included.

## Insecure?

It seems like the authors of `hyper-reverse-proxy` did put effort into keeping it secure, however this is utility crate was only put together to proxy servers on `localhost`. Others can take this code and put the effort into maintaining a reverse proxy intended to be used in more precarious situations if they'd like.
