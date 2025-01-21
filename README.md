# Ramon

> :warning: This project is a WIP and not yet functional. All configuration is subject to change.

Ramon is a lightweight, versatile server monitoring framework. It runs actions (e.g. sending emails) when certain conditions are met. For example, you can configure Ramon to send an email every time an SSH connection from a new IP address is established. Or send an email when a service goes down. Perhaps you would like daily emails of any 5xx server errors that occurred. Ramon makes this easy.

Ramon's design is heavily inspired by [fail2ban](https://github.com/fail2ban/fail2ban) and [Tasker](https://play.google.com/store/apps/details?id=net.dinglisch.android.taskerm).

## Examples

### Setup

```toml
[notify.default]
smtp = "localhost:587"
from = "ramon@example.com"
to = "you@example.com"
limit = "10/m"
# When a notification is dispatched, wait 10 seconds to see
# if another notification with the same id is dispatched,
# and aggregate them into one notification. If this process
# repeats for longer than 1 minute, send the notification immediately.
aggregate = "10s"
aggregate_timeout = "1m"
notify = "default"

# Aggregate all info notifications, and send them all in
# one email at 8:00AM daily.
[notify.info]
aggregate = "0 8 * * *"

# Do not aggregate critical notifications.
[notify.critical]
aggregate = 0
```

### SSH

```toml
# Log each login from a new IP
[monitor.ssh_login]
service = "ssh"
match_log = '^.*\]: Accepted \S+ for (?<user>\S+) from (?<ip>)'
unique = "ip"
notify = { type = "critical", title = "New SSH login from {{ip}} to {{user}}@{{host}}" }
```

### Nginx

```toml
[var]
nginx_log = "/var/log/nginx/access.log"

[monitor.nginx_5xx]
log = "{{nginx_log}}"
match_log = '^\S+ \S+ \S+ \[.+\] "(?<path>.*)" (?<code>5\d{2})'
notify = { type = "error", title = "Server error: {{code}} at {{path}}" }
```

### HTTP

```toml
[monitor.example_endpoint]
every = "5m"
get_fail = "https://example.com/endpoint"
notify = { type = "error", title = "{{url}}: {{err}}" }
```

### systemd

```toml
[monitor.services]
on = [ "service_fail" ]
notify = { type = "error", title = "Service failed: {{service}}" }

[monitor.critical_service]
service = "criticald"
on = [ "service_fail" ]
notify = { type = "critical", title = "Critical daemon failed! Restarting..." }
```

### System integrity

```toml
[monitor.etc_passwd]
watch = "/etc/passwd"
notify = { type = "critical", title = "File changed: /etc/passwd" }

[monitor.ports]
on = [ "port_open" ]
notify = { type = "critical", title = "New port opened: {{port}}" }
```

## Specification (WIP)

On startup, Ramon loads [an internal config file] with sane defaults, and then it loads /etc/ramon.d/\*.toml, and finally it loads /etc/ramon.toml. Each succeeding config file overwrites any properties loaded prior.\*

### Monitors

Monitors are configured by creating a table in the `[monitor]` table (e.g. `[monitor.example]`). Each key in a monitor is classified as an event, a condition, or an action. A monitor must have at least one event. When an event is fired, the monitor evaluates each condition, and if they are all true, then the actions are performed. Monitors can share data with each other through variables.

### Events

#### `service` string

This event is fired every time the specified services output a line to the systemd journal.

##### Local variables

- `service` name of the service

#### `log` file (string)

This event is fired for every line that is appended to the specified files.

#### `watch`\* glob (string), or array of globs

This event is fired each time the contents of a file change.

##### Local variables

- `file` the path to the file that changed

#### `every` duration (string)

This event is fired immediately, and then at the specified interval. A value of `"1ms"` fires every millisecond, `"1s"` every second, `"1m"` minute, `"1h"` hour, `"1d"` day, `"1w"` week, and `"1mon"` fires every month.

```toml
[monitor.timestamp]
every = "1m"
notify = { title = 'The current timestamp is {{ exec("date", "+%s") }}.' }
```

#### `at`\* cron (string)

This event is fired at the specified date and time. Refer to <https://crontab.guru> for help.

#### `on`\* string or array of strings

This key allows the monitor to listen to one or more of the following events:

- `service_fail` is fired when a systemd service fails. Sets `service` to the failing service's name.
- `port_forwarded` is fired when a new port is open on your public IP address. Sets `port` (number) to the opened port.
- `port_closed` is fired when a public port is closed. Sets `port` (number) to the closed port.

### Conditions

Conditions are evaluated sequentially in order of priority. Higher priority (least negative) conditions are evaluated before lower priority conditions. The priority is listed in brackets after the key.

#### `cooldown` [-10] duration (string)

This condition is true if actions have not been run within the specified duration.

```toml
[monitor.1]
every = "1s"
cooldown = "1m"
exec = "echo I will never run more than once per minute."
```

#### `match_log` [-20] regex (string)

This condition is true if the line matches the specified regular expressions. This condition only applies to events from `log` or `service`. If this key is an array, all regular expressions must match.

Named capture groups defined in the regular expression will become available as local variables to the following conditions and actions.

#### `ignore_log` [-21] regex (string)

This condition is true if the line does not match the specified regular expression. This condition only applies to events from `log` or `service`.

#### `unique` [-30] variable (string)

This condition is true if the specified variable has not been seen before. Ramon will cache these values in a text file at `/var/cache/ramon/unique_<monitor name>`.

##### Local variables

- `err` description of first error
- `status` number or array of numbers that correspond with the URLs

#### `if`\* [-50] string

This condition allows you to compare different values.

#### `threshold` [-90] string

This condition is true if every preceding condition has been true at least `n` times within `d` duration. The format of this key is `"n/d"`.

```toml
[monitor.server_errors]
log = "/var/log/server/error.log"
threshold = "3/1m"
notify = { title = "Three server errors occured within one minute!" }
```

### Actions

Actions are run when an event fires and all conditions are true.

#### `exec` string or array of strings

This action spawns a child process. If this key is a string, it's passed as an argument to `sh -c` (\*nix) or `cmd /C` (Windows)\*, and variables are passed to the child through the environment. If this key is an array, the first item is the binary, and the remaining items are passed as arguments; variables can be passed to the child as arguments via templates.

> :information_source: Note: Processes are assumed to be short-lived; they will not be killed when Ramon exits.

#### `notify` table or string

This action sends a notification via email, PushBullet, etc. If this key is a string, it is treated as the title, and it's sent without a body. If this key is a table, it can have the following keys:

- `type` the configuration to use (default: `"default"`)
- `title` the title of the notification (default: `"Ramon Notification"`)
- `body` the body

## Notifications\*

\* Not yet implemented
