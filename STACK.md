# Lecteur universel — stack technique (document de référence)

État vérifié dans le code de la **v0.4.1** (20/11/2026) : versions dans `Cargo.toml`, comportements dans `src/`. L'étude initiale (choix des API) est dans [ETUDE-STACK.md](ETUDE-STACK.md) ; ce fichier-ci fait foi.

**Vocabulaire**
- **Socle** : la crate Rust commune aux applis egui de l'auteur (dépôt public `Desarrh/socle`). Elle fournit le thème, les icônes vectorielles, les composants (palette Ctrl+K, autocomplétion, toasts), le décodage d'images, l'instance unique, la mise à jour, les réglages, le journal de plantage et le mode test. C'est une bibliothèque compilée dans l'exe, pas une dépendance à l'exécution.
- **Explorateur** : l'explorateur de fichiers maison (`C:\Dev\explorateur`), qui ouvre le lecteur. **Tube** : le client YouTube maison (`C:\Dev\tube`), qui partage du code vidéo avec le lecteur. **Vigie** (gestionnaire de tâches) et **Launcher** (lanceur de jeux) sont les autres applis de la famille.
- **Tube nommé** : le canal Windows (*named pipe*) de l'instance unique, à ne pas confondre avec l'app Tube.
- **L1** : chercher une phrase dans une vidéo (dans ses sous-titres). **L2** : points forts automatiques des enregistrements de jeu.
- **Règle n°0** : pendant les tests, ne jamais déranger l'utilisateur (fenêtre invisible et sans focus, aucune entrée simulée, pas de son, réglages intacts).

## 1. But et direction

### Problème
Sous Windows, ouvrir, retoucher un peu et convertir un fichier demande une appli différente par type (Photos, Films et TV, lecteur PDF du navigateur, Bloc-notes, petits outils de coupe, convertisseurs en ligne). Elles sont lentes à s'ouvrir ou envoient les fichiers sur Internet.

### Objectifs
- Ouvrir image, PDF, Word, texte/code, audio et vidéo dans une seule fenêtre à onglets, les modifier légèrement et les **convertir en un clic**.
- **Local** : aucun fichier ne quitte la machine ; seule la vérification de mise à jour contacte GitHub.
- **Léger** : un seul exe d'environ 16 Mo (budget : toute fonction qui ajoute plus de 0,5 Mo se justifie), sans installeur ni runtime.
- **Rapide** : première image en 70 à 150 ms (mesuré par `LECTEUR_TRACE`, release, machine de dev) ; jamais plus de 4 ms de calcul sur le fil principal (§3, avec ses exceptions).
- **Jamais destructif** : les conversions sont écrites à côté de l'original sous un nom non encore utilisé (`nom (2).ext`) ; un remplacement explicite garde une sauvegarde.

### Principes (moyens)
- **Windows fait le travail** : Media Foundation, Windows.Data.Pdf et WIC ; aucun ffmpeg ni pdfium embarqué.
- **Interface rendue sur le CPU** : le GPU ne sert qu'à la vidéo.
- Même apparence que l'explorateur (thèmes crème et bleu nuit du socle). Interface en français.

### Non-objectifs
Pas d'édition de PDF ni de Word (lecture et conversion seulement), pas de sortie WEBM, pas de pistes de sous-titres intégrées aux MKV/MP4, pas d'OCR dans la v0.4 (piste B8), pas d'installeur, pas de version macOS ou Linux.

## 2. Stack

| Couche | Choix |
|---|---|
| Langage | Rust 1.99.1 (MSVC), édition 2024 |
| UI | `egui` 0.37.0, `egui-winit` 0.37.0 (presse-papiers et liens), `winit` 0.30.13, `egui_extras` 0.37 (coloration syntaxique sans syntect) |
| Rendu | **CPU** : `hdrshot-egui-backend` 0.2.0 (importé sous le nom `egui_software_backend`) + `softbuffer` 0.4.8 (affichage GDI) |
| Socle | `socle` **v0.1.4** (git, tag épinglé). Features : `ui`, `images`, `img-exr`, `img-svg`, `img-wic`, `settings`, `crash`, `instance`, `update`, `testmode`. Sans `img-jxl` (voir §9) |
| Vidéo | Media Foundation `IMFMediaEngine`, qui dessine **directement sur le GPU dans une sous-fenêtre** (`vidwin.rs`). Replis : décodage DXVA par D3D11 avec relecture CPU, puis décodage logiciel avec bitmap WIC |
| Montage, conversion audio/vidéo | Media Foundation : Source Reader et Sink Writer, encodeurs H.264, AAC, MP3 et WMA de Windows |
| PDF | Affichage : `Windows.Data.Pdf` (WinRT). Texte : `lopdf` 0.47. Écriture : générateur maison (`pdfgen.rs`, `flate2`) |
| Images | Décodage par `socle::images` : `image` 0.25.10 (png, jpeg, gif, webp, bmp, ico, tiff, tga, dds BC1-3, hdr, qoi), PSD, aperçu RAW, SVG (`resvg`, sans texte), EXR, puis **WIC** en repli (HEIC, AVIF, JPEG XR, JPEG XL, RAW complet, DDS). Écriture : `webp` 0.3 (avec perte), `gif` 0.14 (GIF optimisés) |
| Word | `zip` 9 et `quick-xml` 0.38 (lecteur maison) ; Word par PowerShell + COM seulement s'il est installé, pour un PDF fidèle |
| Audio | `hound` 3.5 (WAV) |
| Données | `serde` / `serde_json` (réglages, cache des points forts, API de mise à jour) |
| Windows | `windows` 0.62.2 (Win32, WinRT, D3D11), `windows-future` 0.3, `drag` 2.1.1 (glisser vers l'extérieur), `winresource` (icône de l'exe) |
| Dépendances à l'exécution | Aucune. `curl.exe` (fourni avec Windows) sert aux mises à jour. Les codecs HEVC, AV1, HEIF, JPEG XL et RAW viennent des extensions du Microsoft Store |
| Polices | `socle::theme::install_fonts_with(bold: true)` : Segoe UI, Segoe UI Bold, Consolas et Segoe UI Symbol en secours ; **pas** de Segoe UI Emoji (13 Mo de RAM évités) |
| Build | `cargo build --release` → `target\release\lecteur-universel.exe`, 15,7 Mo (lto="thin", strip). Par rapport à 12,0 Mo avant le socle : SVG +1,1, EXR +0,7, formats `images` +1,1, `ui` +0,35, `update` et `testmode` le reste. `--features dev` ajoute les outils de développement (§5). Le zip de release est construit dans `dist\` |

## 3. Architecture

### Fichiers

| Fichier | Rôle |
|---|---|
| `main.rs` | Boucle winit, rendu softbuffer, copie de la vidéo en mode CPU, plein écran, instance unique, aperçu rapide, modes en ligne de commande et modes de test (§5) |
| `app.rs` | État : réglages, onglets et vues (`ImgV`, `TxtV`, `PdfV`, `MedV`, Word), chargement des fichiers, `tick()`, raccourcis clavier, actions (convertir, couper, capturer, enregistrer), mises à jour, autotest (`--features dev`) |
| `app_ui.rs` | Tout le dessin : onglets, barre d'état, accueil, vues, **frise de montage**, barre de lecture, panneau Modifier / Convertir / Infos, toasts, vue épurée |
| `commands.rs` | Palette **Ctrl+K** et aide **F1** : commandes selon l'onglet actif, exécution par identifiant |
| `editor.rs` | Éditeur texte/code (recherche, remplacement, aller à la ligne, aperçu Markdown) ; `SearchAc`, l'autocomplétion des champs de recherche |
| `kind.rs` | Type de fichier par extension, couleurs des pastilles, noms non encore utilisés, fichiers voisins, formatage des tailles et des durées |
| `media.rs` | Lecteur Media Foundation (trois modes de rendu) ; lecture du son en flux (`pcm_reader`, `pcm_stream`) pour la forme d'onde, le niveau sonore et le MP3 ; décodage PCM complet, WAV, coupe MP3 sans réencodage, images vidéo pour les GIF |
| `moments.rs` | Points forts (L2), voir §6 |
| `montage.rs` | Vignettes de la frise, coupe vidéo sans réencodage (copie des échantillons), réencodage H.264/AAC |
| `vidwin.rs` | Sous-fenêtre vidéo : traversée par la souris, région découpée sous les éléments d'interface |
| `convert.rs` | Graphe à pivots (voir « Flux ») |
| `pdf.rs` | Rendu des pages PDF dans un fil (file bornée), texte des pages |
| `pdfgen.rs` | Écriture de PDF (texte, Word, images, mode « document scanné »), export Word par COM |
| `docx.rs` | Lecture d'un .docx (paragraphes, styles, listes, tableaux, images), export TXT, MD ou HTML |
| `subs.rs` | Sous-titres SRT / VTT : lecture, réplique à l'instant t, recherche (L1) |
| `wic.rs` | `open_any` (décodage complet par le socle) et aperçu réduit rapide par WIC |
| `clip.rs` | Image vers le presse-papiers (DIB) |
| `winx.rs` | Boîte « Ouvrir », inscription « Ouvrir avec » |
| `update.rs` | Mise à jour depuis GitHub, par `socle::update` |
| `smtc.rs` | Panneau multimédia de Windows et touches multimédia, créé au premier son ou vidéo |
| `testgen.rs` | (`--features dev`) génère le dossier `tests/` |
| socle (crate externe) | Thème (`socle::theme`), icônes (`socle::icons`), composants (`socle::ui`), images, instance, mise à jour, réglages, plantages, mode test |

### Flux
Un fichier est chargé selon son type (`kind`) ; l'onglet garde sa vue. À chaque trame, `tick()` sonde les lecteurs, les vignettes, les pages PDF et les travaux en cours, puis `draw()` dessine. Le dessin renvoie des actions, exécutées après.

Conversions : chaque format d'entrée est ramené à un **pivot**, d'où partent les sorties. Raster = une image en mémoire ; PCM = du son décodé ; Frames = une suite d'images (vidéo → GIF) ; Stream = des échantillons compressés copiés tels quels (coupe sans réencodage) ; Doc = le contenu structuré d'un Word ou d'un PDF ; Text = du texte brut. L'app décrit l'entrée par une `Source` (chemin, type, image ou texte modifiés, plage choisie) et l'envoie à `convert::run`.

### Processus et instances
- **Fenêtre de travail** : instance unique « Lecteur » (`socle::instance` : mutex + tube nommé). Un deuxième lancement transmet ses fichiers, en chemins absolus, et s'arrête.
- **Aperçu rapide** : instance séparée « LecteurApercu », un fichier à la fois, vue épurée. Fermée, elle se cache et reste en veille (CPU 0) 10 min. Après une vidéo, elle est remplacée par une instance de veille neuve, pour rendre le pilote graphique que Windows garde chargé (détails dans `main.rs` ; alternative au §9).
- Pourquoi deux instances : l'aperçu ne doit jamais toucher aux onglets ni aux réglages de l'utilisateur.

### Fils
- Fil principal : interface et événements.
- Un fil par PDF ouvert : rend les pages à la demande (au plus les 8 demandes les plus récentes, la plus récente d'abord).
- Fils ponctuels, annulés quand l'onglet se ferme ou change de fichier : décodage d'image, forme d'onde, vignettes vidéo, points forts, texte PDF, lecture d'un Word.
- Travaux (conversions, coupes, capture en mode GPU, mise à jour) : un à la fois, avec progression ; un panic devient une erreur affichée.
- **Règle** : jamais plus de 4 ms de calcul sur le fil principal ; le reste part dans un fil, avec annulation, et le résultat est gardé en cache. **Exception connue** : l'enregistrement d'une image ou d'un texte (`save_current`) encode et écrit sur le fil principal (plusieurs centaines de ms pour une grosse photo) ; voir la dette §8.
- Les fils réveillent l'interface par `ctx.request_repaint()`. Media Foundation rappelle par `IMFMediaEngineNotify` (file `Mutex<Vec>`).

## 4. Direction artistique

Palette, polices, tailles, arrondis, icônes (≈ 100, grille 24, trait 1,8) et composants (`button`, `wide`, `trunc`, `section_header`, `segmented`, palette, toasts) viennent du **socle** (`socle::theme`, `socle::icons`, `socle::ui`) : les modifier change aussi l'explorateur et les autres applis.

Propre au lecteur :
- **Couleurs de la vue média** (`app_ui.rs`) : `LIGHT` `#F5EDDC` (icônes sur fond sombre), `BAR` `#0F1B2D` (barre de lecture en thème sombre et en plein écran), `MUTED_DARK` `#9FB0C8`, **`GOLD` `#F2B84B`** (sélection de coupe, poignées, bouton « Couper »). Bandes autour de la vidéo : `soft` en clair, `#0B1524` en sombre, noir en plein écran.
- **Repères de la frise** : orange (`live`) pour les points forts (L2), bleu (`accent`) pour les phrases trouvées (L1).
- **Pastilles de type** (`kind::color`) : image `#2F7D5B`, PDF et vidéo `#B5462F`, Word `#2E5E9E`, texte `#D9822B`, audio `#7A4FA0`.
- **Dimensions** : barre d'onglets 42 px ; barre d'état 26 px ; panneau de droite 290 px (320 px à partir de 1300 px, replié sous 900 px) ; barre de lecture vidéo 166 px (temps 30, frise 62, transport 52).
- **Règles** : placements exacts (`scope_builder` + `Rect`) pour que rien ne se chevauche ; tout libellé variable est tronqué avec « … » ; les infos-bulles portent les raccourcis ; jamais d'emoji dans les boutons.

## 5. Conventions

### Raccourcis

| Contexte | Touche | Action |
|---|---|---|
| Partout | Ctrl+O · Ctrl+W · Ctrl+Tab / Ctrl+Maj+Tab | ouvrir · fermer l'onglet · onglet suivant / précédent |
| Partout | Ctrl+K · F1 | palette de commandes · tous les raccourcis |
| Partout | Tab · F ou F11 · Échap | vue épurée · plein écran · sortir du plein écran |
| Partout | PgPréc / PgSuiv | fichier voisin du dossier |
| Partout | touches multimédia | lecture / pause, fichier suivant / précédent, même quand la fenêtre est en arrière-plan (SMTC) |
| Audio, vidéo | Espace ou K | lecture / pause |
| Audio, vidéo | ← → (Maj : ±1 s) · J / L | ±5 s · ±10 s |
| Audio, vidéo | ↑ ↓ · Début · Maj+Début | volume ±10 % · début · début de la sélection |
| Vidéo | , . | image vidéo précédente / suivante |
| Audio, vidéo | I / O (ou [ ]) · X | début / fin de coupe ici · tout garder |
| Vidéo | C · V · G / H | capture PNG · sous-titres oui / non · décalage des sous-titres −/+ 0,1 s |
| Vidéo | Ctrl+F | **L1** : chercher une phrase dans les sous-titres ; Entrée / Maj+Entrée = réplique suivante / précédente |
| Vidéo | N · Maj+N · E | **L2** : point fort suivant / précédent · exporter ±10 s autour |
| Pilule de vitesse | clic · clic droit · double-clic | plus vite · moins vite · 1× |
| Image | ← → · + − 0 1 · R / Maj+R · Ctrl+Z | fichier voisin · zoom (0 ajusté, 1 = 100 %) · pivoter · annuler |
| Image | Ctrl+S · Ctrl+Maj+S | enregistrer une copie · remplacer l'original |
| Texte, code | Ctrl+S · Ctrl+Maj+S | enregistrer le fichier · enregistrer une copie |
| Texte, code | Ctrl+F · Ctrl+H · Ctrl+G · Ctrl+E | rechercher · remplacer · aller à la ligne · aperçu Markdown |
| PDF | Ctrl+F · ← → · + − | rechercher · page précédente / suivante · zoom |
| Champ de recherche | Tab · ↑ ↓ · Entrée | compléter · choisir une suggestion · valider |
| Aperçu rapide | Échap, Espace ou la croix · ← → | cacher l'aperçu · fichier voisin (Espace ne lance pas la lecture : clic ou K) |

La référence vivante est l'aide **F1** (générée par `commands.rs`) ; ce tableau et celui du README la résument. Ctrl+S n'a pas le même sens pour une image et un texte : pour une image, l'action sûre (une copie) est la plus simple à taper ; pour un texte, on enregistre son propre fichier.

### Réglages et données
- `%APPDATA%\lecteur-universel\settings.json` (`socle::settings`, écriture atomique, `#[serde(default)]`) : thème, récents, positions de reprise (au plus 300, la moins récemment utilisée est retirée), taille de la fenêtre, volume, panneau masqué, mise à jour automatique, palette récente, recherches récentes. Écrit à la fermeture ; jamais par l'aperçu ni en test.
- **Sauvegardes** : avant chaque remplacement d'un original, une copie horodatée va dans `Documents\Lecteur universel\Sauvegardes` ; si la copie échoue, rien n'est remplacé. Les sauvegardes de plus de 30 jours sont supprimées sans prévenir (pas de limite de taille du dossier). Aucune sauvegarde si rien n'a changé. L'écriture du nouveau fichier est atomique (fichier temporaire puis renommage).
- Cache des points forts : `%LOCALAPPDATA%\lecteur-universel\points-forts`.
- Journal de plantage : `%APPDATA%\lecteur-universel\crash.log` (socle), signalé par un toast au lancement suivant.

### Ligne de commande
- `lecteur-universel.exe [fichiers…]` : avec un argument (« Ouvrir avec »), vue épurée.
- `--apercu <fichier>` · `--fermer-apercu` · `--quitter-apercu` : ouvrir, cacher, ou terminer l'aperçu rapide. `--apercu-veille <pid>` : interne (recyclage).
- `--convert <ext> <fichiers…>` (avec `LECTEUR_SCAN=1` : PDF « document scanné ») · `--assembler-pdf sortie.pdf [--scan] <images…>`.
- `--apres-maj` : interne (relance après une mise à jour).
- Avec `--features dev` seulement : `--selftest <dossier>` (14 scénarios), `--cut rapide|precis|720p <fichier> <début> <fin> <sortie>`, `--generer-tests <dossier> [largeur hauteur ips secondes]`, `--generer-partie <dossier>` (vidéo de jeu de 60 s avec trois explosions et ses sous-titres), `--thumbs <vidéo>` (mesure des vignettes).

### Mode test et variables
- **Mode discret** (règle n°0), automatique avec `LECTEUR_SHOT`, `LECTEUR_SOLO`, `LECTEUR_DISCRET` ou `SOCLE_TEST` : fenêtre invisible et jamais active (`socle::testmode`), son coupé, réglages jamais écrits, pas de vérification de mise à jour, noms d'instance factices (`LECTEUR_TUBE=<suffixe>`).
- `LECTEUR_SHOT=<png>` (capture du rendu après `LECTEUR_SHOT_MS`, 2 500 par défaut, puis sortie ; état dans `<png>.txt`) · `LECTEUR_EVENTS` (actions injectées dans egui, jamais dans Windows) · `LECTEUR_SIZE`, `LECTEUR_PANELS`, `LECTEUR_RAIL`, `LECTEUR_DARK` · `LECTEUR_TRACE=<fichier>` (étapes du démarrage en ms, pid) · `LECTEUR_FPS` · `LECTEUR_GPU=0|relecture` (forcer un mode vidéo) · `LECTEUR_PDF_TEXTE` (extraire le texte PDF à l'ouverture).

### Protocole d'instance
Un autre programme passe par l'exe (`lecteur-universel.exe --apercu <fichier>`), qui transmet en moins d'1 ms et gère la course au démarrage. Le format interne du tube (`\\.\pipe\<Nom>-<USERNAME>-s<session>-socle-v2`, message `SOCLE1` + arguments séparés par `\0`) appartient au socle et peut changer.

### Installation et mise à jour
- Zip GitHub `Lecteur-vX.Y.Z-windows-x64.zip` (exe, README, licence), avec une empreinte SHA-256 par asset (calculée par GitHub). Sur la machine de dev, `dist\` sert à deux choses : construire les zips de release et héberger l'exe installé (l'explorateur le cherche à côté de lui, dans `dist\`, puis dans `target\release\`).
- Vérification au plus une fois par 24 h (`curl.exe`, API GitHub) ; un bandeau propose la mise à jour. Le socle n'installe un zip que s'il vient de `github.com` en HTTPS et que son SHA-256 correspond.
- Installation : l'exe en cours est renommé `.old`, le nouveau prend sa place, l'aperçu en veille est terminé (`--quitter-apercu`), puis relance avec `--apres-maj`. Le `.old` est supprimé au démarrage suivant. Désactivable dans le menu ⋯.

### Développer avec le socle local
Renommer `.cargo/config.toml.socle-local` en `.cargo/config.toml` (non versionné) : Cargo prend alors `../socle`. Ne jamais commiter le `Cargo.lock` produit ainsi : il doit pointer vers le tag git.

### Code
Commentaires et messages en français ; fonctions de dessin libres (`fn draw_x(ui, &mut vue, &Palette)`) ; actions différées ; `unsafe` limité aux appels COM et Win32 ; `Result<_, String>` pour les messages destinés à l'utilisateur ; erreurs en toast (4 s, 8 s pour une erreur) ou dans la vue (`View::Error`) ; jamais de panique sur un fichier invalide.

## 6. Fonctionnalités

- **Images** (png, jpg, webp, gif animé, bmp, ico, tiff, tga, qoi, hdr, dds, psd, svg, exr ; HEIC, AVIF, JPEG XR, JPEG XL et RAW par WIC, avec l'extension du Store)
  - Grosses photos : aperçu réduit en ~80 ms, pleine définition ensuite ; les modifications attendent la pleine définition.
  - Zoom, déplacement, damier pour la transparence ; pivoter, miroir, rogner (règle des tiers), redimensionner, annuler (8 étapes, ~512 Mo au plus).
  - Conversion vers **PDF** (A4, plusieurs images en un PDF, mode « document scanné »), PNG, JPG, WEBP (≤ 16 383 px), BMP, TIFF, ICO (proportions gardées), GIF.
- **PDF** : pages en continu, zoom, reprise de la page ; recherche Ctrl+F avec autocomplétion ; copier le texte d'une page ; conversion en PNG, JPG, WEBP, TXT, HTML.
- **Word (.docx)** : titres, gras, italique, listes, tableaux, images ; conversion en PDF (sans Word, images comprises), TXT, Markdown, HTML.
- **Texte et code** (environ 40 extensions) : coloration, numéros de ligne, rechercher et remplacer (avec autocomplétion), aller à la ligne, aperçu Markdown ; fins de ligne Windows conservées ; au-delà de 8 Mo, tronqué en lecture seule.
- **Audio** (mp3, wav, m4a/aac, flac, wma, ogg) : forme d'onde lue en flux, poignées de coupe, vitesse 0,25× à 2×, boucle ; extrait MP3 sans réencodage ; conversion en MP3 (en flux) ou WAV.
- **Vidéo** (mp4, mov, m4v, mkv, avi, wmv, webm selon les codecs installés)
  - Lecture GPU directe (mesuré : H.264 5120×1440 à 60 i/s, aucune image perdue, GPU NVIDIA).
  - Frise de vignettes avec poignées, image par image, vitesse, capture PNG (fichier et presse-papiers).
  - Coupe rapide sans réencodage ou à l'image près ; conversion en MP4, GIF, MP3, WAV, PNG.
  - Sous-titres .srt / .vtt (à côté de la vidéo ou glissés dessus ; fichiers de plus de 20 Mo refusés).
  - **L1, chercher une phrase** : Ctrl+F cherche dans les sous-titres, sans tenir compte des accents ni de la casse, avec autocomplétion ; chaque réplique trouvée devient un repère bleu sur la frise.
  - **L2, points forts** (vidéos de plus de 20 s, pas dans l'aperçu rapide) : le niveau sonore est lu en flux par fenêtres de 50 ms ; un point fort est un maximum local (± 0,5 s) au moins 10 dB au-dessus de la moyenne des 10 s précédentes et au-dessus de −35 dBFS ; au plus 40, espacés d'au moins 8 s. Calcul dans un fil annulable, mis en cache par fichier. Repères orange ; N / Maj+N pour sauter, E pour exporter ±10 s.
- **Partout** : onglets, glisser-déposer entrant et sortant, conversion par lot, palette Ctrl+K, reprise de position, récents, vue épurée, plein écran, thèmes clair et sombre, touches multimédia, aperçu rapide, mise à jour automatique.

## 7. Intégrations

- **Explorateur** — existant : ouverture (`lecteur-universel.exe <chemin>`), aperçu rapide à la barre d'espace (`--apercu <chemin>`) et menu « Convertir en… » (`--convert <ext>`). Il cherche `lecteur-universel.exe` (ou `Lecteur.exe`) à côté de lui, dans `dist\`, puis dans `target\release\`. Envisagé : « Assembler en PDF » (`--assembler-pdf`). **Couplage** : l'explorateur garde sa propre liste des conversions possibles, recopiée de `convert::targets` et déjà divergente (voir §8).
- **Tube** : `vidwin` et `smtc` sont encore copiés entre les deux applis (voir §8).
- **Vigie, Launcher** : rien ; le Launcher pourrait ouvrir un fichier dans le lecteur.

## 8. Risques, limites et parades

### Sécurité
| Risque | Parade |
|---|---|
| Un autre processus de la session envoie des arguments au tube d'instance | Seuls des chemins de fichiers à ouvrir et les drapeaux d'aperçu sont acceptés ; aucune conversion ni écriture ne se déclenche par le tube. Tube propre à l'utilisateur et à la session (socle v0.1.4) |
| Mise à jour piégée | Téléchargement seulement depuis `github.com` en HTTPS, empreinte SHA-256 vérifiée avant de remplacer l'exe (socle). **Risque restant** : l'empreinte vient de la même API que le zip ; elle protège d'un fichier corrompu, pas d'un compte GitHub compromis (pas de signature de code) |
| Injection de commande par un nom de fichier dans l'export Word (PowerShell accepte aussi ’ ‘ ‚ ‛ comme guillemets) | Les chemins passent par des variables d'environnement, jamais dans le texte du script ; Word arrêté s'il dépasse 60 s ; document protégé refusé (mot de passe factice) au lieu d'une boîte invisible |
| Fichier piégé (« bombe zip » dans un .docx, image géante) | Lectures bornées : 64 Mo de texte et 50 Mo par image dans un .docx, 8 Mo pour un texte, 20 Mo pour des sous-titres ; décodeurs d'images du socle durcis (v0.1.4) ; un panic dans un travail devient une erreur |
| Écriture dans le registre | Seulement HKCU, et seulement quand l'utilisateur clique « Ajouter à "Ouvrir avec" » |

### Fiabilité et ressources
- **Perte de données** : remplacement atomique et sauvegarde préalable (voir §5). Réglages écrits à la fermeture seulement : un plantage perd les derniers récents et positions.
- **RAM vidéo** : environ 600 Mo pour une vidéo 5K à 60 i/s (tampons du décodage GPU), environ 850 Mo pendant le calcul des vignettes. La fenêtre de travail garde le pilote graphique (≈ 90 Mo, mesuré) après une vidéo ; seul l'aperçu le rend.
- **Mode GPU direct** : la vidéo est une sous-fenêtre au-dessus de l'interface ; chaque élément posé dessus doit être déclaré comme zone à découper (`occl` : barre du haut, toasts, menus, palette, aide, suggestions). Un oubli le cache sous la vidéo.
- **Formats** : pas de sortie WEBM (Windows n'a pas d'encodeur VP9). HEIC, AVIF, JPEG XL demandent une extension du Store (HEIC non testé sur la machine de dev). Pas de PNM.
- **PDF** : la recherche saute à la page sans surligner le mot ; pas de sélection à la souris ; rien sur les PDF scannés.
- **Word → PDF sans Word** : mise en page simple (polices standard).
- **L1** ne cherche que dans les sous-titres externes. **L2** se fie au son seul (une musique forte compte aussi) ; le cache n'est pas versionné.
- **GIF animés** : textures limitées à 200 Mo.
- **Fils sans ordonnanceur commun** : chaque PDF ouvert a son fil et son document WinRT (pas de limite du nombre d'onglets PDF) ; forme d'onde, vignettes, points forts et texte PDF tournent en parallèle sans priorité. Piste : un pool borné avec priorité à l'onglet visible.
- **Moteur vidéo dans le processus** : la fenêtre de travail ne rend jamais le pilote graphique ; piste : un processus enfant jetable pour la vidéo (coûts : protocole, focus, plein écran).
- **Texte de plus de 8 Mo** : lecture seule ; piste : texte par morceaux et mise en page des seules lignes visibles.

### Dette
- `app_ui.rs` et `app.rs` font environ 2 000 lignes chacun ; un fichier par vue (`view_image.rs`, `view_media.rs`…) les allégerait.
- `vidwin` et `smtc` restent copiés entre le lecteur et Tube (candidats pour le socle).
- L'enregistrement (`save_current`) tourne sur le fil principal : à passer en travail de fond.
- La liste des conversions possibles est recopiée dans l'explorateur ; parade envisagée : `lecteur-universel.exe --formats <ext>` que l'explorateur interroge.
- `pdfgen.rs` et `docx.rs` sont des briques maison (voir §9).
- Pas de tests unitaires : `--selftest` (14 scénarios : édition d'image et de texte, coupes MP3 et vidéo, capture, lot WEBP, points forts, niveau sonore, recherche et lecture des sous-titres, Word piégé, échappement PDF, formats d'un PDF) et le passage sur les fichiers de `tests/`.

## 9. Alternatives écartées

| Choix retenu | Alternative | Pourquoi écartée |
|---|---|---|
| Media Foundation | ffmpeg | +40 à 80 Mo d'exe ou de DLL ; Windows décode déjà H.264, HEVC, AV1 (avec les extensions) et a les encodeurs. Coût accepté : pas de WEBM ni de pistes de sous-titres intégrées |
| Windows.Data.Pdf + lopdf | pdfium | +5 à 10 Mo et une DLL ; Windows rend les PDF. Coût accepté : pas de surlignage ni de sélection précise (B4) |
| egui rendu sur le CPU | wgpu / glow | environ 150 Mo de RAM réservés par le pilote pour un contexte GPU permanent (mesure de l'étude initiale), démarrage plus lent ; la vidéo a déjà sa sous-fenêtre GPU. (Les ≈ 90 Mo du §8 sont autre chose : ce que le pilote garde après la fermeture d'une vidéo) |
| `egui_extras` (coloration) | syntect | +3 à 5 Mo pour une coloration plus riche, inutile pour lire et retoucher |
| JPEG XL par WIC | `img-jxl` (jxl-oxide) | +2,2 Mo pour un format rare ; Windows 11 le lit avec l'extension du Store |
| `curl.exe` pour la mise à jour | client HTTP intégré (TLS) | +1 à 2 Mo ; `curl.exe` est fourni avec Windows 10 et 11 |
| Instance unique par tube nommé (socle) | fichiers déposés dans un dossier ; FindWindow + `WM_COPYDATA` | le dossier impose un sondage permanent ; `WM_COPYDATA` demande une fenêtre déjà créée et ne règle pas la course au démarrage, que le mutex + tube gère ; le tube dort à 0 % et livre en moins d'1 ms |
| Recyclage de l'aperçu après une vidéo (instance neuve) | l'aperçu se termine simplement après une vidéo | gain : la veille reste à ~28 Mo au lieu de garder ~90 Mo de pilote, et l'aperçu suivant ne repaie pas le démarrage. Mais un démarrage à froid mesuré (66 à 145 ms) est proche d'un aperçu à chaud (59 à 100 ms) : la fin simple est une **simplification candidate**, à trancher sur mesure |
| Générateur PDF maison (`pdfgen.rs`) | `printpdf` ; imprimante « Microsoft Print to PDF » | contrôle exact de la page A4, du mode « document scanné » et des images JPEG copiées sans réencodage, quelques centaines de lignes ; l'imprimante Windows passe par le spouleur (lent, boîte de dialogue, non scriptable proprement) |
| Lecteur .docx maison (`docx.rs`) | `docx-rs` | seuls titres, styles simples, listes, tableaux et images sont lus ; le lecteur maison borne les lectures (bombe zip) et reste petit. Coût accepté : mise en page simplifiée |

## 10. Pistes (non faites, à valider)

Toutes suivent la règle des 4 ms du §3. Effort : S = quelques heures, M = une à deux journées, L = plus.

| # | Piste | Comment | Effort |
|---|---|---|---|
| A4 | **« Copier comme… »** (Ctrl+Maj+C) : copier le fichier ouvert dans un autre format sans rien enregistrer (HEIC → PNG pour Discord, Word → Markdown, PDF → texte) | conversion en mémoire dans un fil, toast « Copié » ; formats légers seulement | S |
| A5 | **Versions d'un fichier texte** : faire défiler les sauvegardes du lecteur, lignes changées surlignées (sans git, qui ajouterait une dépendance) | sauvegardes listées en différé, différences dans un fil | M |
| A6 | **Papier officiel prêt à envoyer** : détection des bords de la feuille, redressement, rendu « document scanné », nom de fichier proposé à partir du texte lu (en attente : dépend de l'OCR, B8) | bords sur une image réduite (~600 px, durée estimée < 20 ms), OCR Windows ensuite | M |
| A7 | **Refaire la même chose sur le fichier suivant** (Ctrl+R) | rejoue la dernière suite d'actions ; en attente d'un besoin répété constaté | S |
| B4 | **Surligner et sélectionner le texte des PDF** | `lopdf` ne donne pas les positions des glyphes : il faut interpréter les flux de contenu et les polices | L |
| B7 | **Exporter pour Discord / X en moins de 10 Mo** | coupe sans réencodage si ça tient ; sinon Sink Writer H.264 de Media Foundation (encodeur matériel s'il existe, quel que soit le fabricant) avec débit calculé | M |
| B8 | **OCR** des photos et PDF scannés | `Windows.Media.Ocr`, dans un fil, page visible d'abord, résultat en cache | M |

Écartée : **A1, conversion devinée au survol de la souris** — gain de quelques centaines de ms sur des conversions déjà rapides, au prix de calculs spéculatifs, de fichiers temporaires et d'une copie si le dossier est sur un autre disque que `%TEMP%`. À reconsidérer seulement si une mesure montre une attente gênante.
