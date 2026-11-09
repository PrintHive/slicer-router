/* C declarations for crates/slicer-router-ffi/src/lib.rs. Keep the two in step. */
#ifndef SLICER_ROUTER_H
#define SLICER_ROUTER_H

#include <stddef.h>
#include <stdint.h>

/* What to draw for one .3mf. Release it with sr_thumbnail_free. */
typedef struct SrThumbnail {
    /* The .app of the slicer that will open the file, or NULL when Slicer
       Router would ask which slicer to use. Its icon is the badge. */
    char *opener_app;
    /* The preview image the file carries, PNG or JPEG, or NULL. */
    uint8_t *image;
    size_t image_len;
} SrThumbnail;

/* Route `file` with the snapshot the app saved under `home`, the user's real
   home folder, which may be NULL. */
SrThumbnail sr_thumbnail(const char *file, const char *home);

void sr_thumbnail_free(SrThumbnail thumbnail);

#endif
