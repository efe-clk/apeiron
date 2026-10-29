/* Backs up a folder through the C ABI and prints the result (docs/SDK.md §2.2).
 * Usage: example <new-repository> <folder> */
#include <stdio.h>

#include "apeiron.h"

static int fail(void) {
    fprintf(stderr, "%s\n", apeiron_last_error());
    return 1;
}

int main(int argc, char **argv) {
    if (argc != 3) {
        fprintf(stderr, "usage: %s <new-repository> <folder>\n", argv[0]);
        return 2;
    }
    if (apeiron_abi_version() >> 16 != APEIRON_ABI_VERSION >> 16) {
        fprintf(stderr, "wrong library version\n");
        return 1;
    }
    apeiron_repo *repo = NULL;
    if (apeiron_repo_init(argv[1], &repo) != 0) {
        return fail();
    }

    char request[4096];
    /* The folder must not need JSON escaping in this short example. */
    snprintf(request, sizeof request, "{\"source\": \"%s\", \"ref\": \"main\"}", argv[2]);
    apeiron_job *job = NULL;
    if (apeiron_start(repo, "backup", request, &job) != 0) {
        apeiron_repo_free(repo);
        return fail();
    }
    char *result = NULL;
    int32_t status;
    while ((status = apeiron_job_wait(job, 100, &result)) == APEIRON_PENDING) {
        char *progress = NULL;
        if (apeiron_job_progress(job, &progress) == 0) {
            fprintf(stderr, "%s\n", progress);
            apeiron_string_free(progress);
        }
    }
    apeiron_job_free(job);
    if (status != 0) {
        apeiron_repo_free(repo);
        return fail();
    }
    printf("%s\n", result);
    apeiron_string_free(result);

    if (apeiron_call(repo, "list", "{\"snapshot\": \"main\"}", &result) != 0) {
        apeiron_repo_free(repo);
        return fail();
    }
    printf("%s\n", result);
    apeiron_string_free(result);
    apeiron_repo_free(repo);
    return 0;
}
