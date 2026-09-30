# Kuriero

A blocking client for the Telegram Bot API. Kuriero is Esperanto for courier.

`Client::send` posts one of the crate's method bodies as JSON and reads the result as
the type the method names. `Client::request` takes any body ureq can send, such as a
multipart upload. A call Telegram refuses with a rate limit or a failure of
its own is made again, up to three attempts, after the wait Telegram names or a
doubling backoff. Every other refusal ends the call at once as `Error::Rejected`.
A retry can send a message twice when the answer to the first attempt was lost, so a
method that sends a message reads its answer as `Sent`, the message id alone.

`Client::get_updates` polls for messages and button presses.

The `rustls` feature, on by default, lets the client reach an `https` API server. A
bot talking to a local Bot API server over plain HTTP builds without it.

## Checks

See `./check.sh`.
