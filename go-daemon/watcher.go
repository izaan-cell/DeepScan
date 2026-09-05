package main

import (
	"context"
	"log"
	"os"
	"path/filepath"
	"strings"

	"github.com/fsnotify/fsnotify"
	"google.golang.org/grpc"

	pb "github.com/izaan-cell/DeepScan/go-daemon/pb"
)

// Watcher recursively tracks directories with fsnotify and streams every
// create/modify/delete/rename event to the Rust engine's IndexService.
type Watcher struct {
	fsw    *fsnotify.Watcher
	client pb.IndexServiceClient
}

func NewWatcher(conn *grpc.ClientConn) (*Watcher, error) {
	fsw, err := fsnotify.NewWatcher()
	if err != nil {
		return nil, err
	}
	return &Watcher{fsw: fsw, client: pb.NewIndexServiceClient(conn)}, nil
}

// Directories fsnotify has no business watching one-by-one: dependency/
// build trees and VCS metadata can nest thousands of subdirectories (a
// single Rust or Node project easily has 5,000+), and fsnotify needs one
// OS-level watch handle per directory — walking into these blows through
// the process's file descriptor limit ("too many open files"), which
// crashes the whole daemon rather than just skipping the offending folder.
var skipDirNames = map[string]bool{
	"node_modules": true, "target": true, "dist": true, "build": true,
	"vendor": true, "venv": true, "__pycache__": true, "DerivedData": true,
}

// shouldSkipDir also skips every hidden dot-directory generally (.git,
// .cache, .npm, .Trash, ...) on top of the named build/dependency trees
// above — hidden folders are rarely something a user wants live-watched
// for search, and there are far more of them in practice than any fixed
// list could name.
func shouldSkipDir(name string) bool {
	return skipDirNames[name] || strings.HasPrefix(name, ".")
}

// projectMarkerFiles sit directly inside the root of a software project
// (or its VCS metadata), not a folder of the user's own documents/photos.
// Watching (and later indexing) an entire dev project that happens to live
// on the Desktop or in Documents was exactly what caused DeepScan's own
// source tree — frontend/app.js, packaging assets like a DMG background
// PNG — to show up in place of, or alongside, the user's real files.
var projectMarkerFiles = []string{
	".git", "Cargo.toml", "go.mod", "package.json", "pom.xml",
	"pyproject.toml", "Gemfile", "composer.json", "CMakeLists.txt",
}

// isSoftwareProjectDir reports whether dir is the root of a software
// project/app-bundle rather than user content. Callers skip these
// entirely rather than descending in — an .app bundle's insides are
// exclusively executables/plist metadata, never something a user placed
// there themselves.
func isSoftwareProjectDir(dir string) bool {
	base := filepath.Base(dir)
	if strings.HasSuffix(base, ".app") || strings.HasSuffix(base, ".xcodeproj") || strings.HasSuffix(base, ".xcworkspace") {
		return true
	}
	for _, marker := range projectMarkerFiles {
		if _, err := os.Stat(filepath.Join(dir, marker)); err == nil {
			return true
		}
	}
	return false
}

// AddRecursive walks root and registers every subdirectory with fsnotify,
// skipping the heavy/irrelevant directories listed in skipDirNames and any
// nested software-project directory (see isSoftwareProjectDir) — except
// root itself, so explicitly watching a code project still works.
// fsnotify only watches the exact directories it's given, so new
// directories created later are picked up reactively inside Run.
func (w *Watcher) AddRecursive(root string) error {
	return filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil // skip unreadable subtrees, keep scanning
		}
		if !d.IsDir() {
			return nil
		}
		if shouldSkipDir(d.Name()) {
			return filepath.SkipDir
		}
		if path != root && isSoftwareProjectDir(path) {
			return filepath.SkipDir
		}
		return w.fsw.Add(path)
	})
}

// Run streams fsnotify events to the engine over a long-lived gRPC stream
// until ctx is cancelled.
func (w *Watcher) Run(ctx context.Context) {
	stream, err := w.client.WatchEvents(ctx)
	if err != nil {
		log.Fatalf("failed to open WatchEvents stream: %v", err)
	}

	for {
		select {
		case <-ctx.Done():
			return

		case event, ok := <-w.fsw.Events:
			if !ok {
				return
			}
			// A newly created directory needs its own watch registered
			// (unless it's one of the heavy/irrelevant ones — see
			// skipDirNames above the same reasoning applies here).
			if event.Op&fsnotify.Create == fsnotify.Create {
				if info, err := os.Stat(event.Name); err == nil && info.IsDir() {
					if !shouldSkipDir(filepath.Base(event.Name)) && !isSoftwareProjectDir(event.Name) {
						_ = w.fsw.Add(event.Name)
					}
				}
			}

			if err := stream.Send(&pb.FsEvent{
				Kind: toChangeKind(event.Op),
				File: &pb.FileRef{Path: event.Name},
			}); err != nil {
				log.Printf("failed to stream fs event for %s: %v", event.Name, err)
			}

		case err, ok := <-w.fsw.Errors:
			if !ok {
				return
			}
			log.Printf("fsnotify error: %v", err)
		}
	}
}

func toChangeKind(op fsnotify.Op) pb.ChangeKind {
	switch {
	case op&fsnotify.Create == fsnotify.Create:
		return pb.ChangeKind_CHANGE_KIND_CREATED
	case op&fsnotify.Write == fsnotify.Write:
		return pb.ChangeKind_CHANGE_KIND_MODIFIED
	case op&fsnotify.Remove == fsnotify.Remove:
		return pb.ChangeKind_CHANGE_KIND_DELETED
	case op&fsnotify.Rename == fsnotify.Rename:
		return pb.ChangeKind_CHANGE_KIND_RENAMED
	default:
		return pb.ChangeKind_CHANGE_KIND_UNSPECIFIED
	}
}
