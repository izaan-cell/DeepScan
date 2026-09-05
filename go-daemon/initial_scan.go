package main

import (
	"context"
	"io"
	"log"

	"google.golang.org/grpc"

	pb "github.com/izaan-cell/DeepScan/go-daemon/pb"
)

// triggerInitialScan asks the engine to walk root once at startup — the
// ongoing fsnotify watch (see watcher.go) only reports changes going
// forward, so without this, files that already existed before the daemon
// started would never get indexed at all.
func triggerInitialScan(ctx context.Context, conn *grpc.ClientConn, root string) {
	client := pb.NewIndexServiceClient(conn)
	stream, err := client.IndexPath(ctx, &pb.IndexPathRequest{RootPath: root, Recursive: true})
	if err != nil {
		log.Printf("initial scan of %s failed to start: %v", root, err)
		return
	}

	for {
		progress, err := stream.Recv()
		if err == io.EOF {
			return
		}
		if err != nil {
			log.Printf("initial scan of %s failed: %v", root, err)
			return
		}
		if progress.Done {
			log.Printf("initial scan of %s complete: %d indexed, %d skipped, %d scanned",
				root, progress.FilesIndexed, progress.FilesSkipped, progress.FilesScanned)
		}
	}
}
