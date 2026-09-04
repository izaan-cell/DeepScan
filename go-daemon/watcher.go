package main

import (
	"context"
	"log"
	"os"
	"path/filepath"

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

// AddRecursive walks root and registers every subdirectory with fsnotify.
// fsnotify only watches the exact directories it's given, so new directories
// created later are picked up reactively inside Run.
func (w *Watcher) AddRecursive(root string) error {
	return filepath.WalkDir(root, func(path string, d os.DirEntry, err error) error {
		if err != nil {
			return nil // skip unreadable subtrees, keep scanning
		}
		if d.IsDir() {
			return w.fsw.Add(path)
		}
		return nil
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
			// A newly created directory needs its own watch registered.
			if event.Op&fsnotify.Create == fsnotify.Create {
				if info, err := os.Stat(event.Name); err == nil && info.IsDir() {
					_ = w.fsw.Add(event.Name)
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
