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

// isSoftwareProjectDir reports whether dir is an app bundle rather than
// user content — its insides are exclusively executables/plist metadata,
// never something a user placed there themselves, so it's excluded
// wholesale regardless of what's inside it.
//
// This used to also treat any directory containing a `.git`,
// `package.json`, `Cargo.toml`, etc. as "a software project" and exclude
// the whole thing — the actual goal was keeping DeepScan's own source
// tree out of search, but checking for "looks like any software project"
// excluded every git-tracked project wholesale, including the user's own
// real code repos, which defeats code search entirely. shouldSkipDir
// above already excludes the actual build/dependency noise
// (node_modules, dist, etc.) regardless of which project it's in.
// bundleExtensions are macOS package/bundle directory extensions — a
// folder Finder shows and treats as a single opaque file, but a plain
// filesystem walk happily descends into. `Photos Library.photoslibrary`
// is the motivating case: a bundle containing thousands of internal
// cache/thumbnail/database files, none of which are "a file the user
// input" — walking into it both pollutes search with cache internals and
// is genuinely slow (thousands of individual files to watch one by one).
var bundleExtensions = []string{
	".app", ".xcodeproj", ".xcworkspace", ".photoslibrary", ".pages", ".key", ".numbers", ".rtfd",
}

func isSoftwareProjectDir(dir string) bool {
	base := filepath.Base(dir)
	for _, ext := range bundleExtensions {
		if strings.HasSuffix(base, ext) {
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
