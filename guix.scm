;;; GNU Guix package definition for Mio.
;;;
;;; Build from a checkout with:
;;;   guix build -f guix.scm
;;; Install in the current Guix profile with:
;;;   guix package --install-from-file=guix.scm

(use-modules (guix base16)
             (guix base32)
             (guix build-system cargo)
             (guix download)
             (guix gexp)
             (guix git-download)
             ((guix licenses)
              #:prefix license:)
             (guix packages)
             (guix utils)
             (gnu packages)
             (ice-9 match)
             (ice-9 rdelim))

(define %mio-source-directory
  (current-source-directory))

(define %mio-smithay-revision
  "f217f62bbe3f5c414997b91d1fe9caeb5e8662d3")

(define (assignment-value line key)
  "Return the Scheme string assigned to KEY in a simple Cargo.lock LINE."
  (let ((prefix (string-append key " = ")))
    (and (string-prefix? prefix line)
         (call-with-input-string (substring line
                                            (string-length prefix)) read))))

(define (cargo-lock-crates file)
  "Read registry crate names, versions, and SHA-256 checksums from FILE."
  (call-with-input-file file
    (lambda (port)
      (let loop
        ((name #f)
         (version #f)
         (source #f)
         (checksum #f)
         (result '()))
        (define (finish result)
          (if (and name version source checksum
                   (string-prefix? "registry+" source))
              (cons (list name version checksum) result) result))
        (let ((line (read-line port)))
          (cond
            ((eof-object? line)
             (reverse (finish result)))
            ((string=? line "[[package]]")
             (loop #f #f #f #f
                   (finish result)))
            ((assignment-value line "name")
             =>
             (lambda (value)
               (loop value version source checksum result)))
            ((assignment-value line "version")
             =>
             (lambda (value)
               (loop name value source checksum result)))
            ((assignment-value line "source")
             =>
             (lambda (value)
               (loop name version value checksum result)))
            ((assignment-value line "checksum")
             =>
             (lambda (value)
               (loop name version source value result)))
            (else (loop name version source checksum result))))))))

(define (hex-sha256->nix-base32 checksum)
  (bytevector->nix-base32-string (base16-string->bytevector checksum)))

(define (locked-crate-input crate)
  (match crate
    ((name version checksum)
     (list (string-append "rust-" name "-" version)
           (origin
             (method url-fetch)
             (uri (crate-uri name version))
             (file-name (string-append name "-" version ".crate"))
             (sha256 (base32 (hex-sha256->nix-base32 checksum))))))))

(define %mio-cargo-inputs
  ;; Cargo.lock is the source of truth, just as it is for the Nix package.
  ;; Keeping this generated at package-evaluation time avoids maintaining a
  ;; second 250-entry dependency list by hand.
  (map locked-crate-input
       (cargo-lock-crates (string-append %mio-source-directory "/Cargo.lock"))))

(define %mio-source-file?
  (git-predicate %mio-source-directory))

(define %mio-smithay-source
  (origin
    (method git-fetch)
    (uri (git-reference (url "https://github.com/Smithay/smithay.git")
                        (commit %mio-smithay-revision)))
    (file-name (git-file-name "smithay"
                              (string-take %mio-smithay-revision 7)))
    (sha256 (base32 "18ny08lqdj3g68l82g2nmif2x733fb032ycp79qsd0p9ngj3p66i"))))

(define mio
  (package
    (name "mio")
    (version "0.1.0")
    (source
     (local-file %mio-source-directory
                 "mio-checkout"
                 #:recursive? #t
                 #:select? %mio-source-file?))
    (build-system cargo-build-system)
    (arguments
     (list
      #:cargo-build-flags ''("--release" "--workspace" "--ignore-rust-version")
      #:cargo-test-flags ''("--workspace" "--ignore-rust-version")
      #:install-source? #f
      #:phases
      #~(modify-phases %standard-phases
          (add-after 'unpack 'use-packaged-smithay
            (lambda* (#:key inputs #:allow-other-keys)
              (let ((smithay (assoc-ref inputs "smithay-source")))
                (substitute* "Cargo.toml"
                  (((string-append
                     "smithay = \\{ git = \"https://github.com/Smithay/smithay.git\", "
                     "rev = \""
                     #$%mio-smithay-revision "\","))
                   (string-append "smithay = { path = \"" smithay "\","))
                  (((string-append
                     "smithay-drm-extras = \\{ git = \"https://github.com/Smithay/smithay.git\", "
                     "rev = \""
                     #$%mio-smithay-revision "\" \\}"))
                   (string-append "smithay-drm-extras = { path = \"" smithay
                                  "/smithay-drm-extras\" }"))))))
          (replace 'install
            (lambda* (#:key outputs #:allow-other-keys)
              (let ((bin (string-append (assoc-ref outputs "out") "/bin")))
                ;; The workspace was already built and tested above.  Copy its
                ;; release artifacts instead of making `cargo install` resolve
                ;; and compile the same dependency graph for a second time.
                (install-file "target/release/mio-compositor" bin)
                (install-file "target/release/mioctl" bin))))
          (add-after 'install 'install-mio-data
            (lambda* (#:key outputs #:allow-other-keys)
              (let* ((out (assoc-ref outputs "out"))
                     (bin (string-append out "/bin"))
                     (share (string-append out "/share"))
                     (session (string-append bin "/mio-session")))
                (install-file "config/mio.kdl"
                              (string-append share "/mio"))
                (call-with-output-file session
                  (lambda (port)
                    (format port
                     "#!~a~%exec ~a/mio-compositor --backend udev \"$@\"~%"
                     (which "sh") bin)))
                (chmod session #o755)
                (mkdir-p (string-append share "/wayland-sessions"))
                (call-with-output-file (string-append share
                                        "/wayland-sessions/mio.desktop")
                  (lambda (port)
                    (format port
                            (string-append "[Desktop Entry]~%"
                             "Name=Mio~%"
                             "Comment=The Mio Wayland compositor~%"
                             "Exec=~a~%"
                             "Type=Application~%"
                             "DesktopNames=mio~%") session)))))))))
    (native-inputs
     (append %mio-cargo-inputs
             `(("pkg-config" ,(specification->package "pkg-config")))))
    (inputs `(("smithay-source" ,%mio-smithay-source)
              ("wayland" ,(specification->package "wayland"))
              ("libdrm" ,(specification->package "libdrm"))
              ("libdisplay-info" ,(specification->package "libdisplay-info"))
              ("mesa" ,(specification->package "mesa"))
              ("libinput" ,(specification->package "libinput-minimal"))
              ("libseat" ,(specification->package "libseat"))
              ("eudev" ,(specification->package "eudev"))
              ("libxkbcommon" ,(specification->package "libxkbcommon"))))
    (home-page "https://github.com/bakumugi777/mio-wm")
    (synopsis "Wayland compositor with one continuous two-dimensional world")
    (description
     "Mio is a Rust and Smithay Wayland compositor and tiling window manager.
Windows live in one continuous two-dimensional world, and the display acts as a
camera looking into that world.")
    (license license:expat)))

mio
