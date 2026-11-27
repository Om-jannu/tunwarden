// Captured from tunwarden 0.1.0 on 27 Nov 2026 with a demo config: the
// tunnels were fake-tunnel processes and the google login was a stand-in
// script, so the output is the real CLI's, the cloud is not. Column spacing
// is as printed; only the stand-in script paths are shown as they were.
export const capturedOn = "27 Nov 2026";

export const expired = `$ tunwarden status
NAME           STATE    LOCAL           PID    UPTIME  RESTARTS  DETAIL
bastion-redis  running  127.0.0.1:6380  47594  16s     0
prod-api       failed   127.0.0.1:8080  -      -       1         auth expired: the google login has expired; run \`tunwarden login google\`
staging-db     failed   127.0.0.1:5433  -      -       1         auth expired: the google login has expired; run \`tunwarden login google\``;

export const login = `$ tunwarden login google
Running the google login (used by staging, prod): /tmp/twdemo/login.sh
You are now logged in as [me@example.com].
logged in; restarting prod-api, staging-db`;

export const recovered = `$ tunwarden status
NAME           STATE    LOCAL           PID    UPTIME  RESTARTS  DETAIL
bastion-redis  running  127.0.0.1:6380  47594  21s     0
prod-api       running  127.0.0.1:8080  47683  3s      1
staging-db     running  127.0.0.1:5433  47682  3s      1`;

export const prodGuard = `$ tunwarden start prod-api
error: refusing to start production tunnels (prod-api) without confirmation
hint: pass --yes to confirm`;

export const config = `version = 1

[[logins]]                        # something you renew now and then
name = "google"
check = "gcloud config config-helper --force-auth-refresh --format=none"
login = "gcloud auth login me@example.com"
expired = ["Reauthentication failed"]

[[profiles]]                      # environment, production flag and login
name = "staging"
login = "google"

[[tunnels]]                       # any command that opens a local port
name = "staging-db"
profile = "staging"
command = "gcloud compute start-iap-tunnel db-1 5432 --zone=europe-west1-b --local-host-port=127.0.0.1:5433"
groups = ["daily"]
forwards = [{ local_port = 5433 }]

[[tunnels]]                       # SSH forwards are built for you
name = "bastion-redis"
type = "ssh-local"
host = "bastion"
forwards = [{ local_port = 6380, remote_host = "redis.internal", remote_port = 6379 }]`;
