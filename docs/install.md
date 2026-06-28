# Installation HXLinux (testeurs)

> Version anglaise : [install.en.md](install.en.md)

## Télécharger

Récupérez la dernière release sur GitHub : **Releases** → asset **AppImage** ou **.deb** (Linux x86_64).

Tag conseillé pour la première version utilisable : `v0.1.0` (pré-release).

## Installer

### AppImage (recommandé — toutes distros)

```bash
chmod +x hxlinux_*_amd64.AppImage
./hxlinux_*_amd64.AppImage
```

### Debian / Ubuntu (.deb)

```bash
sudo dpkg -i hxlinux_*_amd64.deb
hxlinux
```

## Accès USB (obligatoire)

Sans règle udev, l’app ne verra pas le Helix (ou il faudra lancer en `sudo`).

```bash
sudo cp packaging/99-line6-helix.rules /etc/udev/rules.d/
sudo udevadm control --reload-rules && sudo udevadm trigger
```

Débranchez/rebranchez le Stomp XL. Votre utilisateur doit être dans le groupe `plugdev` (défaut sur Ubuntu).

## Prérequis

| Élément | Détail |
|---------|--------|
| OS | Linux x86_64 (testé Ubuntu/Debian) |
| Hardware | **HX Stomp XL** (seul modèle validé pour l’édition complète) |
| HX Edit | **Non requis** pour l’usage normal — métadonnées modèles incluses dans l’installable |

## Premier lancement

1. Brancher le Stomp XL en USB.
2. Lancer HXLinux — la fenêtre presets + la matrice models s’ouvrent.
3. Attendre la connexion (pastille verte) et le chargement du preset actif.

## Limites connues (v0.1)

- Helix LT / Floor : détection possible, édition non supportée.
- Réordonner la liste presets : UI seulement (pas encore envoyé au HX).
- Load preset depuis disque : non implémenté.
- Voir [features-v1.md](features-v1.md) pour l’inventaire complet.

## Compiler soi-même

```bash
git clone https://github.com/Scrouik/hxlinux.git
cd hxlinux
npm ci
npm run tauri build
```

Artefacts dans `src-tauri/target/release/bundle/`.

Dépendances build (Debian/Ubuntu) :

```bash
sudo apt install libwebkit2gtk-4.1-dev libayatana-appindicator3-dev librsvg2-dev patchelf libudev-dev
```
