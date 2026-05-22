import type { paths } from './types';

type ResponseBody<Response> = Response extends {
  content: { 'application/json': infer Body };
}
  ? Body
  : undefined;

type HealthzSuccess = ResponseBody<
  paths['/healthz']['get']['responses']['204']
>;
type ReadyzSuccess = ResponseBody<paths['/readyz']['get']['responses']['200']>;
type ReadyzUnavailable = ResponseBody<
  paths['/readyz']['get']['responses']['503']
>;

type HailApiErrorBody<Status extends number> = Status extends 503
  ? ReadyzUnavailable
  : unknown;

export class HailApiError<Status extends number = number> extends Error {
  readonly name = 'HailApiError';

  constructor(
    readonly status: Status,
    readonly body: HailApiErrorBody<Status>,
    readonly response: Response,
  ) {
    super(`hail API request failed with HTTP ${status}`);
  }
}

export class HailApiClient {
  readonly #baseUrl: URL;

  constructor(opts: { baseUrl: string }) {
    this.#baseUrl = new URL(opts.baseUrl);
  }

  async getHealthz(): Promise<HealthzSuccess> {
    const response = await this.#request('/healthz');

    if (response.status === 204) {
      return undefined as HealthzSuccess;
    }

    throw await this.#error(response);
  }

  async getReadyz(): Promise<ReadyzSuccess> {
    const response = await this.#request('/readyz');

    if (response.status === 200) {
      return undefined as ReadyzSuccess;
    }

    if (response.status === 503) {
      throw await this.#error<503>(response);
    }

    throw await this.#error(response);
  }

  async #request(pathname: keyof paths): Promise<Response> {
    const url = new URL(pathname, this.#baseUrl);

    return fetch(url, {
      credentials: 'include',
      headers: {
        accept: 'application/json',
      },
    });
  }

  async #error<Status extends number = number>(
    response: Response,
  ): Promise<HailApiError<Status>> {
    return new HailApiError(
      response.status as Status,
      (await readResponseBody(response)) as HailApiErrorBody<Status>,
      response,
    );
  }
}

async function readResponseBody(response: Response): Promise<unknown> {
  if (response.status === 204) {
    return undefined;
  }

  const text = await response.text();
  if (text.length === 0) {
    return undefined;
  }

  const contentType = response.headers.get('content-type') ?? '';
  if (contentType.includes('application/json')) {
    return JSON.parse(text);
  }

  return text;
}
