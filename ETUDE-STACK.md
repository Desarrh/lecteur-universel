# Lecteur universel — étude de stack

Objectif : petit exe, démarrage instantané, peu de RAM. Règle : **ce que Windows sait faire, on le délègue à Windows** (Media Foundation, WinRT, WIC). Pas de ffmpeg, pas de pdfium embarqués.

## Socle (repris de l'explorateur)

| Besoin | Choix | Pourquoi |
|---|---|---|
| UI | `egui` 0.37 + `egui-winit` + `winit` 0.30 | Même code/thème que l'explorateur, partage possible plus tard |
| Rendu | `softbuffer` + `hdrshot-egui-backend` (CPU, GDI) | **Pas de contexte GPU** → évite les ~150 Mo du pilote NVIDIA. L'explorateur fait déjà ça (exe 8 Mo) |
| API Windows | `windows` (WinRT + Media Foundation + COM) | Une seule crate, features à la carte |
| Glisser vers l'extérieur | `drag` | Déjà validé dans l'explorateur |
| Réglages / reprise | `serde_json` dans `%APPDATA%\lecteur-universel` | Positions, récents, thème |
| Instance unique + onglets | mutex nommé + `WM_COPYDATA` | Un 2ᵉ double-clic ajoute un onglet au lieu d'ouvrir une 2ᵉ fenêtre |
| « Ouvrir avec » | clés `HKCU\Software\Classes\Applications\lecteur.exe` | Sans droits admin |

## Par format

| Format | Lire / afficher | Modifier | Convertir | Coût en taille |
|---|---|---|---|---|
| **Images** png/jpg/webp/gif/bmp | `image` (déjà utilisée) ; WIC en secours (HEIC, TIFF…) | rogner/pivoter/redimensionner : `image` | png ↔ jpg ↔ webp (`image`, WebP **sans perte** seulement) | ~1,5 Mo |
| **PDF** | `Windows.Data.Pdf` (WinRT) → rendu page en bitmap | — | PDF → PNG par page ; PDF → TXT plus tard (`lopdf`) | **0** |
| **Word .docx** | Parseur maison `zip` + `quick-xml` → paragraphes, titres, gras/italique, listes, tableaux, images | texte simple (v2) | → TXT / Markdown / HTML : facile. → **PDF : voir risques** | ~0,5 Mo |
| **Texte / code** | `egui::TextEdit` + `syntect` (jeu de langages réduit) | oui (Ctrl+S, copie de secours) | → PDF/HTML plus tard | ~2 Mo |
| **MP3 / WAV / AAC / FLAC** | Media Foundation (décodage + lecture) ; forme d'onde via Source Reader → PCM | coupe **sans réencodage** (trames MP3, octets WAV), fondus = réencodage | WAV : `hound` ; MP3/AAC : encodeurs MF intégrés à Windows | ~0,1 Mo |
| **MP4 / MOV / MKV*** | Media Foundation (`IMFMediaEngine` ou Source Reader), image par image, vitesse 0,25–2× | coupe sans réencodage (Sink Writer en passthrough), retirer le son, pivoter | → MP4 H.264 (MF, accéléré matériel) ; → GIF (`gif` + `color_quant`) ; → MP3 ; capture PNG | ~0,3 Mo |

\* HEVC, AV1, VP9 : dépendent des extensions du Microsoft Store installées. MKV : pris en charge par MF depuis Windows 10.

**Estimation** : exe ≈ **9–12 Mo** (contre +80–100 Mo avec ffmpeg). Démarrage < 200 ms. RAM au repos ≈ 30–50 Mo, plus le fichier ouvert.

## Compromis et risques

1. **Vidéo en rendu CPU.** Afficher du 1080p à 30 i/s via softbuffer, c'est copier ~8 Mo par image : ça passe, mais ça coûte du CPU. Le décodage matériel (DXVA) charge le pilote GPU, donc la RAM qu'on voulait éviter.
   → Proto : décodage logiciel MF + affichage CPU, puis on mesure. Plan B : sous-fenêtre native D3D11 réservée à la vidéo, qui ne charge le GPU que lorsqu'une vidéo est ouverte.
2. **DOCX → PDF fidèle.** Windows n'a pas d'API pour ça.
   - Si Word est installé : automatisation COM (`ExportAsFixedFormat`), fidélité parfaite.
   - Sinon : notre propre rendu → PDF (`printpdf` / `krilla`), fidélité moyenne (polices, tableaux complexes, en-têtes).
3. **WebP avec perte, WEBM, FLAC en sortie.** Pas d'encodeur dans Windows ni dans `image`.
   - WebP avec perte : crate `webp` (libwebp en C, ~300 Ko) si tu y tiens.
   - WEBM : abandonné, ou plus tard.
4. **GIF de qualité.** `gifski` donnerait le meilleur rendu mais il est sous licence AGPL. On prend `color_quant` (MIT), un peu moins joli.
5. **Lecture audio et vidéo synchronisées** : `IMFMediaEngine` gère la synchro, mais son mode « frame server » demande un peu de COM. C'est la partie la plus technique du proto.

## Prototype proposé (dans `C:\Dev\lecteur-universel`)

Périmètre : ouvre **image, PDF, MP3/WAV, MP4**, plus le texte. Fenêtre au design A2 (thèmes crème et bleu nuit), avec :
- les onglets ;
- le glisser-déposer ;
- le passage du chemin en argument (« Ouvrir avec ») ;
- la vue épurée et la touche F pour le plein écran ;
- le rail Modifier / Convertir / Infos (Infos fonctionnel, Convertir pour image→image et wav↔mp3).

Hors proto, pour la v1 : coupe audio/vidéo, docx, conversion par lot, glisser du résultat, reprise de position.
