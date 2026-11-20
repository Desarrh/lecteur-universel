# Lecteur universel — stack technique (document de référence)

État vérifié dans le code de la **v0.3.1** (commit `f38e9c4`, 20/11/2026).
Historique : l'étude initiale (choix des API, compromis) est dans [ETUDE-STACK.md](ETUDE-STACK.md) ; ce fichier-ci fait foi.

## 1. But et direction

- **À quoi ça sert** : ouvrir n'importe quel fichier (image, PDF, Word, texte/code, audio, vidéo), le modifier légèrement et le **convertir en un clic** vers un autre format.
- **Ce que ça remplace** : Photos, Films et TV, le lecteur PDF du navigateur, le Bloc-notes pour le code, les petits outils de coupe audio/vidéo, et les convertisseurs en ligne (ConvertX, VERT…).
- **Principes**
  - **Léger** : un seul exe d'environ 12 Mo, sans installeur ni runtime, démarrage en 0,1 à 0,5 s.
  - **Windows fait le travail** : Media Foundation, Windows.Data.Pdf et WIC. Aucun ffmpeg ni pdfium embarqué.
  - **Interface rendue sur le CPU** : pas de contexte GPU pour l'interface. Le GPU ne sert que lorsqu'une vidéo est ouverte.
  - **Jamais destructif** : les conversions sont écrites à côté de l'original, avec un nom libre (`nom (2).ext`). Seuls les remplacements explicites écrasent, et ils gardent une sauvegarde.
  - **Même apparence que l'explorateur** : mêmes thèmes crème et bleu nuit, même rendu.
  - **Interface en français**, README en anglais.

## 2. Stack

| | |
|---|---|
| Langage | Rust 1.99.1 (MSVC), édition 2024 |
| UI | `egui` 0.37.0, `egui-winit` 0.37.0 (fonctions clipboard et liens), `winit` 0.30.13, `egui_extras` 0.37 (coloration syntaxique sans syntect) |
| Rendu | **CPU** : `hdrshot-egui-backend` 0.2.0 (importé sous le nom `egui_software_backend`) + `softbuffer` 0.4.8 (affichage GDI) |
| Vidéo | Media Foundation `IMFMediaEngine` qui dessine **directement sur le GPU dans une sous-fenêtre** (`vidwin.rs`). Replis : décodage DXVA par D3D11 avec relecture CPU, puis décodage logiciel avec bitmap WIC |
| Montage et conversion audio/vidéo | Media Foundation : Source Reader et Sink Writer, encodeurs H.264, AAC, MP3 et WMA de Windows |
| PDF | Affichage : `Windows.Data.Pdf` (WinRT). Texte : `lopdf` 0.47. Écriture : générateur maison (`pdfgen.rs`, `flate2`) |
| Socle | **`socle` v0.1.4** (dépôt `Desarrh/socle`, dépendance git épinglée par tag ; `Cargo.lock` pointe vers le tag git. Pour développer le socle en même temps, renommer `.cargo/config.toml.socle-local` en `.cargo/config.toml` (non versionné) : Cargo prend alors `../socle`, mais ne jamais commiter le `Cargo.lock` produit ainsi). Polices : `install_fonts_with(bold: true)`, sans emoji (13 Mo de RAM évités). Features : `ui` (thème, icônes, palette Ctrl+K, autocomplétion, toasts), `images` + `img-exr` + `img-svg` + `img-wic`, `settings`, `crash`, `instance`, `update`, `testmode`. **Sans `img-jxl`** (+2,2 Mo) : le JPEG XL passe par WIC (extension « JPEG XL » du Store) |
| Images | Décodage par `socle::images` : `image` 0.25.10 (png, jpeg, gif, webp, bmp, ico, tiff, tga, dds BC1-3, hdr, qoi), PSD, aperçu RAW, SVG (`resvg`, sans texte), EXR, puis **WIC** en repli (HEIC, AVIF, JPEG XR, JXL, RAW complet, DDS BC7). `webp` 0.3 (WebP avec perte) et `gif` 0.14 (GIF optimisés) pour l'écriture |
| Word | `zip` 9 et `quick-xml` 0.38 (lecteur maison) ; PowerShell + COM Word seulement si Word est installé, pour un PDF fidèle |
| Audio | `hound` 3.5 (WAV) |
| Windows | `windows` 0.62.2 (Win32, WinRT et D3D11 ; features dans `Cargo.toml`), `windows-future` 0.3, `drag` 2.1.1 (glisser vers l'extérieur), `winresource` (icône de l'exe) |
| Dépendances externes | Aucune à l'exécution. `curl.exe` (fourni avec Windows) sert pour les mises à jour. Les codecs HEVC, AV1, HEIF et RAW viennent des extensions du Microsoft Store |
| Build | `cargo build --release` produit `target\release\lecteur-universel.exe` (15,6 Mo depuis le socle, contre 12,0 avant : SVG +1,1 Mo, EXR +0,7 Mo, formats `images` +1,1 Mo, composants `ui` +0,35 Mo ; lto="thin", strip). Le zip de release est construit dans `dist\` |

## 3. Architecture

| Fichier | Rôle |
|---|---|
| `main.rs` | Boucle winit, rendu softbuffer, copie de la vidéo en mode CPU, plein écran, instance unique, modes ligne de commande (`--convert`, `--cut`, `--assembler-pdf`, `--generer-tests`, `--selftest`) et modes de test (`LECTEUR_SHOT`, `LECTEUR_EVENTS`, `LECTEUR_FPS`, `LECTEUR_TRACE=<fichier>` qui journalise les étapes du démarrage en ms et le pid). **Mode discret** (règle n°0) : automatique en test, fenêtre invisible et jamais active (`socle::testmode::hide`), son coupé, réglages jamais écrits, pas de vérification de mise à jour, noms d'instance factices (`LECTEUR_TUBE=<suffixe>`) |
| `app.rs` | État : réglages, onglets et vues (`ImgV`, `TxtV`, `PdfV`, `MedV`), chargement des fichiers, `tick()` (sondage des lecteurs, vignettes, PDF), raccourcis clavier, actions (convertir, couper, capturer, enregistrer), mises à jour, autotest |
| `app_ui.rs` | Tout le dessin : onglets, barre d'état, accueil, vues image/PDF/Word, **frise de montage**, barre de lecture, panneau Modifier / Convertir / Infos, toasts (socle, dans la barre d'état ; en bulle en vue épurée), vue épurée |
| `commands.rs` | Palette **Ctrl+K** et aide **F1** (socle) : liste des commandes selon l'onglet actif (fichier, vue, image, lecture, montage, conversions), exécution par id |
| `editor.rs` | Éditeur texte/code : recherche et remplacement, aller à la ligne, numéros de ligne alignés sur l'affichage, aperçu Markdown. `SearchAc` : **autocomplétion** des champs de recherche (mots du document indexés dans un fil + recherches récentes, aussi pour le PDF) |
| `kind.rs` | Type de fichier par extension, couleurs des pastilles, noms de fichier libres, fichiers voisins, formatage des tailles et des durées |
| `media.rs` | Lecteur Media Foundation (trois modes de rendu), décodage PCM, MP3 (encodeur Windows), coupe MP3 sans réencodage, images vidéo pour les GIF |
| `montage.rs` | Vignettes de la frise, coupe vidéo sans réencodage (copie des échantillons), réencodage H.264/AAC |
| `vidwin.rs` | Sous-fenêtre vidéo : traversée par la souris, région découpée sous les éléments d'interface |
| `convert.rs` | **Graphe à pivots** : chaque format d'entrée passe par un pivot (Raster, PCM, Frames, Stream, Doc, Text), puis chaque format de sortie en sort |
| `pdf.rs` / `pdfgen.rs` | Rendu des pages PDF en arrière-plan, texte des pages ; écriture de PDF (texte, Word, images, mode « document scanné ») |
| `docx.rs` | Lecture d'un .docx (paragraphes, styles, listes, tableaux, images) et export TXT, MD ou HTML |
| `subs.rs`, `wic.rs`, `clip.rs` | Sous-titres SRT/VTT et recherche (L1) ; `open_any` (socle), aperçu réduit rapide par WIC, encodeur HEIC ; image vers le presse-papiers (DIB) |
| `winx.rs`, `update.rs` | Boîte « Ouvrir », inscription « Ouvrir avec » ; mise à jour depuis GitHub par `socle::update` (thème, icônes, instance unique, réglages et plantages viennent aussi du socle) |
| `moments.rs` | **Points forts (L2)** : niveau sonore lu en flux (`media::loudness`, 50 ms), montées de plus de 10 dB au-dessus des 10 s précédentes, espacées d'au moins 8 s ; calcul dans un fil annulé à la fermeture de l'onglet, cache `%LOCALAPPDATA%\lecteur-universel\points-forts` |
| `smtc.rs` | Panneau multimédia de Windows et touches multimédia (repris de Tube), créé au premier son ou vidéo |
| `testgen.rs` | Génère le dossier `tests/` (tous les formats, plus une vidéo synthétique à la résolution de l'écran) |

**Flux** : un fichier est ouvert puis chargé selon son type (`kind`). L'onglet garde sa vue. Chaque image d'écran, `tick()` sonde les lecteurs, les vignettes et les pages PDF, puis `draw()` dessine. Les conversions envoient une `Source` au graphe `convert::run`. Les fichiers produits apparaissent dans le panneau et peuvent être glissés vers l'extérieur.

**Threads**
- Fil principal : interface et événements.
- Un thread par PDF ouvert, qui rend les pages à la demande.
- Fils ponctuels : forme d'onde audio, vignettes vidéo, extraction du texte PDF, conversions et coupes, vérification des mises à jour, serveur du tube nommé.
- Les fils de travail réveillent l'interface avec `ctx.request_repaint()`.
- Media Foundation rappelle par `IMFMediaEngineNotify`, et les événements passent par une file `Mutex<Vec>`.

## 4. Direction artistique

Toutes les valeurs ci-dessous sont définies dans **`src/theme.rs`**, sauf mention contraire.

**Palette** (`palette(dark)`, struct `Palette`, identique à l'explorateur) :

| Rôle | Clair (crème) | Sombre (bleu nuit) |
|---|---|---|
| `bg` fond | `#EFE6D3` | `#0F1B2D` |
| `chrome` onglets, barre d'état | `#E4D8BF` | `#0B1525` |
| `surface` panneaux | `#F5EDDC` | `#13223A` |
| `side` | `#EEE4CF` | `#101D31` |
| `code` champs, tuiles | `#F7F0E1` | `#0D182A` |
| `border` | `#D6C8AA` | `#243756` |
| `soft` survol léger, pastilles | `#E8DCC3` | `#1C2D48` |
| `text` | `#2A2620` | `#E7ECF4` |
| `muted` | `#625846` | `#9FB0C8` |
| `accent` / `on_accent` | `#2E5E9E` / `#FFFFFF` | `#6FA8FF` / `#0B1525` |
| `sel_bg` sélection | `#D5DDE6` | `#23406B` |
| `hover_bg` | `#EBE1CB` | `#182A44` |
| `live` / `live_bg` | `#A9561A` / `#F1DFC4` | `#F2A65A` / `#2A2420` |
| `add` / `add_bg` (succès) | `#2F7346` / `#E0EBD6` | `#7FD49C` / `#123024` |
| `del` / `del_bg` (erreur) | `#A83434` / `#F2DCD4` | `#F0A7A7` / `#341C24` |

**Constantes de la vue média** (`app_ui.rs`, en haut du fichier) :
- `LIGHT` `#F5EDDC` (icônes sur fond sombre) ;
- `BAR` `#0F1B2D` (barre de lecture en thème sombre et en plein écran) ;
- `MUTED_DARK` `#9FB0C8` ;
- **`GOLD` `#F2B84B`** : couleur de la sélection de coupe, des poignées et du bouton « Couper ».
- Bandes autour de la vidéo : `p.soft` en clair, `#0B1524` en sombre, noir en plein écran.

**Pastilles de type** (`kind::color`) : image `#2F7D5B`, PDF et vidéo `#B5462F`, Word `#2E5E9E`, texte `#D9822B`, audio `#7A4FA0`.

**Polices** (`install_fonts`, chargées depuis `C:\Windows\Fonts`) :
- Segoe UI pour le texte ;
- Consolas pour le monospace ;
- Segoe UI Bold, sous la famille `FontFamily::Name("bold")` ;
- Segoe UI Emoji et Segoe UI Symbol en secours.

**Tailles de texte** : 13,5 pour le texte courant, les onglets et le code ; 12 à 12,5 pour le secondaire ; 11 à 11,5 pour les étiquettes de section en majuscules ; titres en gras 14 à 14,5 ; 21 à 22 sur l'accueil ; 38 en monospace pour le temps audio.

**Arrondis** :
- widgets 6 (styles egui), menus 8, fenêtres et toasts 10 ;
- tuiles 8 à 9, cartes de l'accueil 9, zone de dépôt 16, pastille de l'accueil 20 ;
- onglets arrondis en haut seulement (`nw/ne` 9) ;
- pilules : rayon égal à la moitié de la hauteur (PDF 22, temps 14) ;
- frise 10.

**Espacements et dimensions** :

| Élément | Mesure |
|---|---|
| Barre d'onglets | 42 px, onglets de 36 px de haut et de 118 à 230 px de large |
| Bandeau de mise à jour | 34 px |
| Barre d'état | 26 px |
| Panneau de droite | 290 px, 320 px si la fenêtre fait au moins 1300 px ; replié sous 900 px (`NARROW`) |
| Marges du panneau | 16 / 14 |
| Barre de lecture vidéo | 166 px : ligne du temps 30, frise 62, transport 52 |
| Boutons de transport | 40 px, lecture 52 px rond (accent) |
| Poignées de la frise | 16 px de large |

**Icônes** (`icons.rs`) :
- 38 icônes vectorielles dessinées au pinceau sur une grille 24×24, trait de 1,8.
- Jamais d'emoji ni de glyphe Unicode dans les boutons : ils ne sont pas centrés.

**Composants réutilisables** :
- `icons::button(ui, icon, taille, Look, info-bulle)` : bouton icône carré, icône à 62 % (50 % à partir de 44 px). `Look { fg, bg, hover, round }`. Les variantes `ghost(&p)` et `on_dark()` sont dans `app_ui.rs`.
- `icons::wide(ui, icon, libellé, raccourci, largeur, primaire, &p)` : bouton large de 36 px (40 px en primaire), avec icône, libellé tronqué et raccourci aligné à droite.
- Dans `app_ui.rs` :
  - `elided()` : texte sur une ligne, coupé par « … » ;
  - `section()` : étiquette en majuscules ;
  - `dashed_rect()` : cadre en pointillés ;
  - `trim_strip()` : frise à poignées ;
  - `transport()` : barre de lecture centrée ;
  - `time_row()` : pilule du temps.
- Panneau à segments Modifier / Convertir / Infos : fond `soft`, segment actif `surface` en gras.

**Règles** :
- placements exacts (`scope_builder` + `Rect`) pour les barres, afin que rien ne se chevauche ;
- tout libellé variable est tronqué avec « … » ;
- les infos-bulles portent les raccourcis.

## 5. Conventions

**Raccourcis**

| Touche | Action |
|---|---|
| Ctrl+O · Ctrl+W · Ctrl+Tab | ouvrir · fermer l'onglet · onglet suivant |
| Tab · F / F11 · Échap | vue épurée · plein écran · sortir |
| PgPréc / PgSuiv | fichier voisin du dossier |
| Espace / K | lecture / pause |
| ← → · J / L | ±5 s (avec Maj : ±1 s) · ±10 s (même convention que Tube) |
| , . | image précédente / suivante |
| I / O ou [ ] · X · Maj+Début | début / fin de coupe · tout garder · retour au début de la sélection |
| Touches multimédia | lecture / pause, fichier suivant / précédent (panneau multimédia de Windows, SMTC), même fenêtre en arrière-plan |
| Échap · Espace · ← → (aperçu) | ferment l'aperçu rapide ouvert par `--apercu` · fichier voisin |
| C | capture vidéo |
| V · G / H | sous-titres · décalage des sous-titres |
| Ctrl+K · F1 | palette de commandes · tous les raccourcis |
| Ctrl+F (vidéo) | **L1** : chercher une phrase dans les sous-titres ; repères bleus sur la frise, Entrée / Maj+Entrée = réplique suivante / précédente |
| N · Maj+N · E | **L2** : point fort suivant / précédent · exporter ce moment (±10 s, coupe rapide) ; repères orange sur la frise |
| Ctrl+F / H / G / E | rechercher (avec autocomplétion : Tab complète, ↑↓ choisit) / remplacer / aller à la ligne / aperçu Markdown (Ctrl+F marche aussi dans les PDF) |
| Ctrl+S / Ctrl+Maj+S | **texte** : enregistrer le fichier / enregistrer une copie — **image** : copie / remplacer |
| R / Maj+R · Ctrl+Z | pivoter · annuler (image) |

**Réglages**
- Fichier `%APPDATA%\lecteur-universel\settings.json`, écrit en JSON par serde, avec `#[serde(default)]` donc tolérant aux champs manquants.
- Contenu : thème, récents, positions de reprise (au plus 300), taille de la fenêtre, volume, panneau masqué, mise à jour automatique et date de la dernière vérification.
- Sauvegardé à la fermeture.

**Sauvegardes** : avant chaque remplacement, l'original est copié dans `%TEMP%\lecteur-universel-sauvegardes\<horodatage>-nom`.

**Code**
- Commentaires et messages en français.
- Fonctions de dessin libres (`fn draw_x(ui, &mut vue, &Palette)`) pour éviter les conflits d'emprunt.
- Actions différées : le dessin renvoie une action, exécutée après le dessin.
- `unsafe` limité aux appels COM et Win32.

**Erreurs**
- `Result<_, String>` pour les messages destinés à l'utilisateur, `windows::core::Result` dans la couche Media Foundation.
- Affichage par un toast (vert ou rouge, 5 s) ou dans la vue (`View::Error`).
- Jamais de panique sur un fichier invalide.

**Lancement**
- `lecteur-universel.exe [fichiers…]`.
- Avec un argument (cas d'« Ouvrir avec ») : vue épurée.
- **Instance unique** par `socle::instance` (mutex + tube nommé) sous le nom « Lecteur » : un deuxième lancement transmet ses fichiers (chemins absolus) à la fenêtre ouverte, qui les ouvre en onglets.
- **Aperçu rapide** : `lecteur-universel.exe --apercu <fichier>`. **Instance séparée** de la fenêtre de travail (nom « LecteurApercu »), qui ne touche ni aux onglets ni aux réglages de l'utilisateur. Vue épurée, un fichier à la fois ; ← → ou PgPréc / PgSuiv ouvrent le fichier voisin (J / L avancent dans une vidéo, K lance la lecture). Échap, Espace ou la croix **cachent** la fenêtre ; l'instance reste en veille (CPU 0, pas de sondage) et quitte seule après 10 min sans aperçu. `--fermer-apercu` referme l'aperçu affiché. Après une vidéo, le pilote graphique reste chargé dans le processus (≈ 90 Mo que Windows ne rend pas) : en se cachant, l'aperçu lance une instance de veille neuve (`--apercu-veille <pid>`, fenêtre cachée, une image dessinée à vide pour être déjà chaude ; elle attend la fin de `<pid>`), puis se termine net (`TerminateProcess`, sinon le démontage de Media Foundation garde 1 à 2 s le verrou d'instance).
- **Protocole** (si un programme veut écrire directement) : tube `\\.\pipe\<Nom>-<USERNAME>-socle-v1`, message UTF-8 `SOCLE1` puis chaque argument précédé de `\0` (ex. `SOCLE1\0--apercu\0C:\chemin\photo.jpg`). Le plus simple reste de lancer l'exe : il transmet en moins d'1 ms.
- **Mesures** (release, socle v0.1.4, mode discret, machine calme) : aperçu à froid 66 à 145 ms de l'appui à la 1re image ; à chaud 59 à 100 ms (11 à 30 ms du tube à l'image) ; aperçu suivant une vidéo 70 à 100 ms. Veille : 27 à 29 Mo réservés, moins de 1 Mo réellement utilisé, CPU 0. Fenêtre de travail : 38 Mo réservés avec une image, 125 Mo avec une vidéo 720p.
- Menu ⋯ › « Ajouter à "Ouvrir avec" » : écrit dans HKCU (ProgID `LecteurUniversel.Fichier`).

**Installation et mise à jour**
- Le zip GitHub `Lecteur-vX.Y.Z-windows-x64.zip` contient l'exe, le README et la licence. Raccourci menu Démarrer : `%APPDATA%\Microsoft\Windows\Start Menu\Programs\Lecteur universel.lnk`.
- Vérification au plus une fois par 24 h, par `curl.exe` sur `api.github.com/repos/Desarrh/lecteur-universel/releases/latest`. Un bandeau propose alors de mettre à jour.
- Installation : l'exe en cours est renommé `.old`, le nouveau prend sa place, puis relance avec `--apres-maj`. Le `.old` est supprimé au démarrage suivant. Le tout est désactivable dans le menu ⋯.
- Git : auteur `Desarrh <175427122+Desarrh@users.noreply.github.com>` (adresse noreply), pas de ligne de co-auteur.

## 6. Fonctionnalités (ce qui marche aujourd'hui)

- **Images** (png, jpg, webp, gif animé, bmp, ico, tiff ; HEIC, AVIF, JPEG XR et RAW via WIC)
  - Zoom, déplacement, damier pour la transparence.
  - Pivoter, miroir, rogner (règle des tiers), redimensionner, annuler.
  - Conversion vers **PDF** (A4, plusieurs images en un seul PDF, mode « document scanné »), PNG, JPG (qualité), WEBP (avec ou sans perte), BMP, TIFF, ICO, GIF.
- **PDF**
  - Pages en continu, zoom, reprise de la page.
  - Recherche Ctrl+F, page par page, avec un extrait.
  - Copier le texte d'une page.
  - Conversion en PNG, JPG ou WEBP (une image par page), en TXT ou en HTML.
- **Word (.docx)**
  - Titres, gras et italique, listes, tableaux, images.
  - Conversion en PDF (sans Word, images comprises), TXT, Markdown, HTML (images intégrées).
- **Texte et code** (environ 40 extensions)
  - Coloration syntaxique, numéros de ligne, ligne courante.
  - Rechercher et remplacer, aller à la ligne, aperçu Markdown, retour à la ligne.
  - Enregistrement avec sauvegarde.
  - Conversion en PDF, HTML ou TXT.
- **Audio** (mp3, wav, m4a/aac, flac, wma)
  - Forme d'onde avec poignées de coupe, vitesse de 0,25× à 2×, boucle sur la sélection.
  - Extrait (MP3 sans réencodage).
  - Conversion en MP3 (128, 192 ou 320 kbit/s) ou en WAV.
- **Vidéo** (mp4, mov, m4v, mkv, avi, wmv)
  - Lecture GPU directe : 5120×1440 à 60 i/s sans perte d'image.
  - Frise de vignettes avec poignées, image par image, vitesse, capture PNG (fichier et presse-papiers).
  - Coupe rapide sans réencodage, ou à l'image près.
  - Sous-titres .srt / .vtt.
  - Conversion en MP4 (taille et qualité), GIF optimisé, MP3, WAV, PNG.
- **Partout**
  - Onglets, glisser-déposer entrant et sortant, conversion par lot.
  - Reprise de position, fichiers récents.
  - Vue épurée, plein écran, thèmes clair et sombre, fenêtre adaptative.
  - Mise à jour automatique.

## 7. Intégrations

- **Explorateur** (`C:\Dev\explorateur`)
  - Il lance `lecteur-universel.exe <chemin>` depuis `target\release\` ou `dist\` : **ne pas renommer l'exe**.
  - L'instance unique garantit qu'un double-clic répété ajoute un onglet au lieu d'ouvrir une nouvelle fenêtre.
  - Possibles :
    - un clic droit « Convertir en… » via `--convert <ext> <fichiers>` ;
    - « Assembler en PDF » via `--assembler-pdf out.pdf [--scan] <images>` ;
  - **Aperçu rapide (fait côté lecteur)** : `--apercu <chemin>`, ou écriture directe du chemin dans le tube `\\.\pipe\lecteur-universel-apercu-v1` quand il existe, ce qui évite les ~20-30 ms de lancement d'un processus.
- **Tube** (`C:\Dev\tube`)
  - Il réutilise déjà la même approche vidéo : `IMFMediaEngine` qui dessine dans une sous-fenêtre (Tube a son propre `vidwin.rs`), même thème, mêmes composants.
  - Ce qui pourrait se partager : la frise de vignettes et la coupe (`montage.rs`, pour extraire un clip d'une vidéo téléchargée), la capture d'image, les sous-titres (`subs.rs`), l'export GIF.
  - Ce code est copié, pas partagé. Une crate commune (thème, icônes, vidwin) éviterait la divergence.
- **Vigie et Launcher** : rien d'existant. Le launcher pourrait lancer le lecteur avec un fichier, et le thème et les icônes s'y réutilisent tels quels.

## 8. Limites connues / dette technique

- **RAM vidéo** : environ 600 Mo pour une vidéo 5K à 60 i/s (tampons du décodage GPU), environ 850 Mo pendant le calcul des vignettes.
- **Formats manquants** : pas de sortie WEBM (Windows n'a pas d'encodeur VP9). HEIC demande l'extension « Extensions vidéo HEVC », absente sur la machine de dev.
- **PDF** : la recherche saute à la page sans surligner le mot. Pas de sélection de texte à la souris. Rien sur les PDF scannés (pas d'OCR).
- **Word → PDF sans Word** : mise en page simple (polices standard).
- **Sous-titres** : seulement les fichiers externes ; les pistes intégrées aux MKV et MP4 sont ignorées.
- **Mode GPU direct** : la capture redécode depuis le fichier (environ 100 ms), et les éléments posés sur la vidéo doivent être déclarés comme zones à découper (`occl`).
- **Fichiers volumineux** : un fichier texte de plus de 8 Mo est tronqué et passe en lecture seule. Les textures des GIF animés sont limitées à 200 Mo.
- **L1** ne cherche que dans les sous-titres externes (.srt / .vtt à côté de la vidéo ou glissés dessus), pas dans les pistes intégrées. **L2** se fie au son seul : une musique forte ou une voix qui crie compte aussi ; pas d'analyse dans l'aperçu rapide (qui reste léger). Le cache des points forts n'est pas versionné : à vider si l'algorithme change.
- **Dette**
  - `app_ui.rs` (1 900 lignes) et `app.rs` (1 450) sont gros ; le dessin des vues mériterait un fichier par type.
  - `vidwin` et `smtc` restent copiés entre le lecteur et Tube (candidats pour le socle).
  - Le code contient des modes de test (`LECTEUR_*`) ; sans effet s'ils ne sont pas activés.
- **Pas de tests unitaires** : `--selftest` (10 scénarios, dont points forts, niveau sonore en flux et recherche dans les sous-titres ; `--generer-partie <dossier>` crée une vidéo de jeu de 60 s avec trois explosions et ses sous-titres) et le passage sur les 50 fichiers de `tests/` servent de vérification.

## 9. Pistes QoL (non implémentées — à valider)

Règle commune : **rien ne doit bloquer l'interface**. Tout calcul de plus de quelques millisecondes part dans un fil de travail. Le résultat s'affiche dès qu'il est prêt, et ce qui a été calculé est gardé en mémoire ou en cache disque. Chaque piste indique comment elle reste instantanée.

Effort : S = quelques heures, M = une à deux journées, L = plus.

### (a) Inédites — n'existent nulle part telles quelles

| # | Piste | Pour qui | Comment ça reste instantané | Effort |
|---|---|---|---|---|
| A1 | **Conversion devinée à l'avance** : dès que la souris survole un format dans « Convertir », le lecteur commence la conversion en arrière-plan. Au clic, le fichier est déjà prêt et simplement déplacé | tous | Fil de travail lancé au survol, annulé si la souris part. Résultat dans `%TEMP%`, puis renommé au clic (pas de recopie). Limité aux conversions rapides (images, texte, coupe sans réencodage) | M |
| A2 | **Points forts automatiques dans les enregistrements de jeu** (fait : L2) : la frise affiche des repères là où le son monte d'un coup (explosions, combats, cris). Clic = saut au moment, et « Exporter ce moment » crée un clip de ±10 s | jeu | Calcul sur le son déjà décodé pour la forme d'onde, réduit à 8 kHz mono : environ 1 s de calcul par heure de vidéo, dans un fil. Repères mis en cache par fichier (taille + date) | M |
| A3 | **Chercher une phrase dans une vidéo** (fait : L1) : Ctrl+F sur une vidéo cherche dans ses sous-titres. Chaque occurrence devient un repère doré sur la frise, et Entrée saute au moment où la phrase est dite | YouTube, films | Répliques en mémoire, triées par temps. Recherche linéaire sur quelques milliers de lignes (moins d'une milliseconde), repères redessinés à chaque image | S |
| A4 | **« Copier comme… »** (Ctrl+Maj+C) : copie le fichier ouvert dans le presse-papiers sous un autre format, sans rien enregistrer. Par exemple HEIC → PNG à coller dans Discord, Word → texte Markdown, image de vidéo → PNG, PDF → texte | tous | Conversion en mémoire dans un fil ; le presse-papiers est rempli dès qu'elle est finie, avec un toast « Copié ». Formats légers seulement (pas de vidéo) | S |
| A5 | **Remonter le temps d'un fichier texte** : un curseur fait défiler les versions précédentes (sauvegardes du lecteur et, si le fichier est dans un dépôt git, l'historique git), comme on fait défiler une vidéo, avec les lignes changées surlignées | dev | Versions listées en différé, contenu lu seulement quand on s'arrête sur une version. Différences calculées dans un fil et gardées en cache (au plus 20 versions en mémoire) | M |
| A6 | **Papier officiel prêt à envoyer** : sur la photo d'un document, détection des bords de la feuille, redressement, rendu « document scanné », puis nom de fichier proposé à partir du texte lu (« Facture EDF 2026-11.pdf ») | papiers | Bords détectés sur une version réduite (environ 600 px, moins de 20 ms). L'OCR Windows tourne en arrière-plan après l'affichage et le nom proposé arrive dès qu'il est prêt. Le PDF est déjà généré en parallèle | M |
| A7 | **Refaire la même chose sur le fichier suivant** : le lecteur retient la dernière suite d'actions faite sur un type de fichier (par exemple « recadrer en 9:16 → exporter MP4 720p »). Sur le fichier voisin, une touche (Ctrl+R) la rejoue | jeu, YouTube | Coût nul : la suite d'actions est gardée en mémoire, et sa reprise lance les mêmes travaux en arrière-plan que d'habitude | S |

### (b) Existantes ailleurs, qu'on n'a pas encore

| # | Piste (référence) | Pour qui | Comment ça reste instantané | Effort |
|---|---|---|---|---|
| B1 | **Aperçu rapide à la barre d'espace** (fait) depuis l'explorateur (QuickLook de macOS) : le lecteur s'ouvre en vue épurée, et Échap le ferme | tous | Le lecteur déjà ouvert reçoit le fichier par le tube nommé (moins de 30 ms mesurés). Au premier lancement, la fenêtre s'affiche avant que le contenu soit chargé (affichage différé) | S |
| B2 | **Lire presque tous les formats d'image** (fait, par le socle) (comme IrfanView ou XnView) : SVG, PSD (image fusionnée), TGA, QOI, HDR/EXR, PNM, plus les formats Windows déjà gérés | tous | Fonctions de la crate `image` (`tga`, `qoi`, `hdr`, `exr`, `pnm`) et `resvg` pour le SVG. Décodage dans un fil, un aperçu réduit affiché d'abord pour les grosses images | S |
| B3 | **Autocomplétion dans les champs de recherche** (fait) (VS Code, navigateurs) : Ctrl+F propose les mots du document dès la 2e lettre, et les recherches récentes | dev, papiers | Index des mots construit dans un fil à l'ouverture du fichier (dictionnaire trié). Une recherche par préfixe dans un tableau trié coûte moins d'une milliseconde | S |
| B4 | **Surligner et sélectionner le texte dans les PDF** (Acrobat, navigateurs) : les résultats de recherche sont surlignés sur la page, et on peut sélectionner du texte à la souris pour le copier | papiers, dev | Positions des mots extraites par `lopdf` dans le fil PDF, une page à la fois, seulement pour les pages visibles, et gardées en cache par page | M |
| B5 | **Touches multimédia et contrôles Windows** (fait) (Spotify, Tube) : lecture, pause et piste suivante même quand la fenêtre n'a pas le focus, avec la vignette dans le volume de Windows | YouTube, musique | API SMTC de Windows et raccourcis globaux, sans aucun calcul (Tube a déjà ce code, à réutiliser) | S |
| B6 | **Palette de commandes Ctrl+K** (fait) (VS Code, Raycast) : taper « webp », « couper ici », « sombre »… | tous | Liste des actions en mémoire, recherche floue en moins d'une milliseconde, rien à charger | M |
| B7 | **Exporter pour Discord / X en moins de 10 Mo** (outils de clips, Medal) : coupe la sélection et calcule le débit pour tenir sous la limite | jeu | Coupe sans réencodage si ça tient déjà, sinon réencodage par l'encodeur de la carte graphique (NVENC) en arrière-plan, avec barre de progression ; la fenêtre reste utilisable | M |
| B8 | **OCR** (Text Extractor de PowerToys, Photos) : rendre le texte d'une photo ou d'un PDF scanné sélectionnable, copiable et cherchable | papiers | `Windows.Media.Ocr` intégré à Windows (pas de modèle à embarquer), lancé en arrière-plan sur la page visible, résultat mis en cache | M |
