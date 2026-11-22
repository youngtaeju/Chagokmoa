<p align="center">
  <img src="docs/assets/banner.en.png" alt="Chagokmoa: Where your little one's days add up, one by one" width="100%">
</p>

<p align="center">
  <a href="README.md">한국어</a> | <b>English</b>
</p>

<p align="center">
  <a href="https://github.com/youngtaeju/Chagokmoa/releases/latest"><img src="https://img.shields.io/github/v/release/youngtaeju/Chagokmoa?label=release" alt="Latest release"></a>
  <a href="LICENSE"><img src="https://img.shields.io/github/license/youngtaeju/Chagokmoa" alt="License"></a>
</p>

## About

A self-hosted baby photo journal for your family. Keep your child's photos, videos, and growth records on a single server you run at home. One server serves one family, with no external cloud, ads, tracking, or subscriptions. Original photos and videos are kept exactly as uploaded.

The name comes from the Korean phrase *chagok-chagok moa duda*, "to gather and stack up neatly, one by one."

## Features

**Photos and videos**

- Upload many files at once and resume interrupted uploads. Duplicate files are skipped.
- HEIC photos and phone videos are supported. Originals are stored byte for byte, while the app shows thumbnails and web-playable versions.
- Timeline by capture date, full-screen viewer, and original downloads.
- Deleted photos go to the trash first, so nothing is lost by accident.

**Family**

- Family members join only through invite links. There are three roles: Admin, Guardian (uploads photos and edits baby info), and Family (views and reacts).
- Hearts, comments, mentions, and favorites.
- Web push notifications for new photos, comments, mentions, stories, and milestones.

**Records**

- Baby profiles with age in days and months. Multiple babies are supported.
- Stories (text and photos), height and weight, and milestones such as rolling over, walking alone, and the first tooth.
- Growth charts with WHO reference curves.

**Browse**

- Find photos by date on the calendar, and revisit memories from a year or a few months ago today.
- Group photos into albums.

**Operations**

- Use it in the browser or install it to the home screen (PWA). Dark mode, Korean, and English.
- Runs on a home server or NAS with one Docker image and one PostgreSQL database.
- Encrypted backups with restic (local folder and remote repository) and restore, configured in the app.
- Optional: SMTP mail (invites, password recovery) and OIDC sign-in (Authentik, Authelia, Keycloak, Google, and others).

## Browser and home screen app

Open your server address in a browser and start using it. Install it as an app (PWA) on your phone or computer to open it full screen without the address bar and to receive push notifications. No app store is involved.

- iPhone, iPad: In Safari, tap Share, then "Add to Home Screen"
- Android: In the Chrome menu, choose Install app (or Add to Home screen)
- Computer: The install icon in the Chrome or Edge address bar

Turn on push notifications per device in Settings → Notifications. On iPhone and iPad, push notifications work only in the app added to the home screen. App installation and push notifications work only over https (or localhost).

### Supported browsers

- Chrome, Edge, Firefox, and Safari released within the last two years
- Safari on iOS 16.4 or later (including the app added to the home screen)
- Chrome on Android

Browsers without `Intl.supportedValuesOf` show only the current value in the time zone picker.

## Installation

Run it on a server with Docker and Docker Compose (a home server, NAS, and so on). The image is `ghcr.io/youngtaeju/chagokmoa` and supports `linux/amd64`. The only external dependency is PostgreSQL, which is included in the compose file.

1. Copy `docker-compose.yml` and `.env.example` from the `compose/` directory to a working directory on your server, rename `.env.example` to `.env`, and fill in the values.

   - `SECRET_KEY`: 64 hex characters generated with `openssl rand -hex 32`.
   - `POSTGRES_PASSWORD`: The database password. It must match the password inside `DATABASE_URL`. Alphanumeric characters are recommended; special characters must be URL-encoded in `DATABASE_URL`.
   - `PUBLIC_URL`: The address you open in the browser. Behind a reverse proxy, use its https address.
   - `PUID`, `PGID`: The host user that owns the `./data` directory. Check them on the host with `id -u` and `id -g`.

2. Start it.

   ```bash
   docker compose up -d
   ```

   Database migrations are applied automatically on startup. Originals and derived files are stored in `./data`, and the database in `./postgres`.

3. Open the `PUBLIC_URL` address (default `http://localhost:8080`) in the browser and sign up. The first account becomes the Admin (owner). Other family members join through invite links created by the Admin.

The compose `ports` setting opens the port on all host interfaces. If a reverse proxy runs on the same host, bind to `127.0.0.1` only.

```yaml
    ports:
      - "127.0.0.1:${PORT:-8080}:8080"
```

For reverse proxies, https, backups, mail, and OIDC sign-in, see the [operations guide](docs/operations.md) (in Korean).

### Upgrading

Change `CHAGOKMOA_VERSION` in `.env` to the new version, then run the command below. If you use the default `latest`, run it without changing the value.

```bash
docker compose pull && docker compose up -d
```

New migrations are applied automatically on startup. Back up before upgrading ([backups](docs/operations.md#백업), in Korean).

## License

Copyright (C) 2026 youngtaeju

AGPL-3.0-only. See [LICENSE](LICENSE).

The Docker image bundles third-party packages, each under its own license. For example, Debian's ffmpeg links GPL components (x264, x265, and others), and the image also includes libheif. Sources for these packages are available from Debian.

The growth chart reference curves embed the WHO Child Growth Standards tables, reduced to monthly values. The tables themselves carry no explicit license, so they are treated under CC BY-NC-SA 3.0 IGO (non-commercial), the license of WHO publications. The source and terms are in [web/src/growth/who/NOTICE.md](web/src/growth/who/NOTICE.md).
